mod error;
mod source_parser;

#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub use error::PkgLoadError;
pub use source_parser::{BiwacSourceParser, SourceParser};

use biwac_ast::{Globals, ModAst, ModDecl};
use biwac_base::{
    BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME, BIWA_EXTENSION, BIWA_LIBRARY_PACKAGE_ROOT_MODULE_NAME,
    ErrorContext, ErrorHolder, IdentInterner, InternedIdent, MetadataHolder, ModId, ModPath,
    ModSource, PackageId, PackageKind, SourceHolder,
};

#[derive(Debug)]
pub struct LoadedModule {
    pub mod_id: ModId,
    pub ast: ModAst,
    pub children: HashMap<InternedIdent, LoadedModule>,
}

impl LoadedModule {
    /// 自身と全子孫モジュールを深さ優先で走査する。
    pub fn walk(&self, f: &mut impl FnMut(&LoadedModule)) {
        f(self);
        for (_, child) in self.children_ordered() {
            child.walk(f);
        }
    }

    /// 子モジュールを決定論的な順序 ([`ModId`] 昇順 = ファイル名順) で返す。
    ///
    /// `children` は名前引きのために HashMap なので、走査順は実行ごとに変わる。
    /// DefId の採番や、順序が成果物に残る処理はこちらを使う。
    /// 採番がぶれると .biwameta がバイト単位で変わり、
    /// SVH による鮮度判定が毎回「変更あり」になってしまう。
    pub fn children_ordered(&self) -> Vec<(&InternedIdent, &LoadedModule)> {
        let mut children: Vec<(&InternedIdent, &LoadedModule)> = self.children.iter().collect();
        children.sort_by_key(|(_, m)| m.mod_id);
        children
    }

    /// 自身と全子孫モジュールを深さ優先で走査する (可変)。
    ///
    /// AST を書き換えるパス (arch による native の刈り込みなど) が使う。
    pub fn walk_mut(&mut self, f: &mut impl FnMut(&mut LoadedModule)) {
        f(self);
        // 走査順は決定論的でなければならない。
        // 子の書き換えが順序に依存しない場合でも、
        // 診断の出る順序が実行ごとに変わると追いにくい。
        let mut ids: Vec<InternedIdent> = self.children.keys().copied().collect();
        ids.sort_by_key(|id| self.children[id].mod_id);
        for id in ids {
            if let Some(child) = self.children.get_mut(&id) {
                child.walk_mut(f);
            }
        }
    }
}

#[derive(Debug)]
pub struct Pkg {
    pub pkg_kind: PackageKind,
    pub root_module: LoadedModule,
}

impl Pkg {
    /// パッケージ内の全モジュールを深さ優先で走査する。
    pub fn walk_modules(&self, mut f: impl FnMut(&LoadedModule)) {
        self.root_module.walk(&mut f);
    }

    /// パッケージ内の全モジュールを深さ優先で走査する (可変)。
    pub fn walk_modules_mut(&mut self, mut f: impl FnMut(&mut LoadedModule)) {
        self.root_module.walk_mut(&mut f);
    }
}

impl Pkg {
    // pkg_root_path はdirであることが保証されている必要がある
    //
    // `P` は 1 ファイルぶんのソースを `ModAst` にする処理 ([`SourceParser`])。
    // 呼び出し元から明示してもらう必要がある (`Pkg::try_load::<BiwacSourceParser>(..)`) —
    // 引数のどこにも `P` が現れないため型推論だけでは決まらない。
    pub fn try_load<'a, P: SourceParser>(
        metadata: &'a MetadataHolder,
        interner: &'a mut IdentInterner,
        srcs: &'a mut SourceHolder, // 空の SourceHolder を受け取る
        pkg_root_path: PathBuf,
    ) -> Result<Self, ErrorHolder<'a, PkgLoadError<'a>>> {
        Self::load::<P>(metadata, interner, srcs, pkg_root_path, false).map(|(pkg, _)| pkg)
    }

    /// [`Self::try_load`] と同じだが、モジュール木の形の誤り
    /// (宣言されていないファイル・ファイルの無い宣言・重複した宣言) では失敗しない。
    ///
    /// そうした誤りは 2 つ目の値として返し、読めたモジュールだけで木を作る
    /// (誤りのある宣言・ファイルは木に入らない)。
    /// エディタ (biwa-lsp) 向け。宣言前の新しいファイルが 1 つあるだけで
    /// パッケージ全体の解析が止まらないようにするため。
    /// ルートモジュールの欠落・構文エラーでは [`Self::try_load`] と同じく失敗する。
    pub fn try_load_tolerant<'a, P: SourceParser>(
        metadata: &'a MetadataHolder,
        interner: &'a mut IdentInterner,
        srcs: &'a mut SourceHolder, // 空の SourceHolder を受け取る
        pkg_root_path: PathBuf,
    ) -> Result<(Self, Vec<PkgLoadError<'static>>), ErrorHolder<'a, PkgLoadError<'a>>> {
        Self::load::<P>(metadata, interner, srcs, pkg_root_path, true)
    }

    fn load<'a, P: SourceParser>(
        metadata: &'a MetadataHolder,
        interner: &'a mut IdentInterner,
        srcs: &'a mut SourceHolder,
        pkg_root_path: PathBuf,
        tolerant: bool,
    ) -> Result<(Self, Vec<PkgLoadError<'static>>), ErrorHolder<'a, PkgLoadError<'a>>> {
        let srcpath = pkg_root_path.join(Path::new("src"));

        let mut ctx = ModuleTreeCtx::new();

        // existence check of root module ( lib.biwa or main.biwa )
        let lib_path = {
            let lib_path = srcpath.join(Path::new(&format!(
                "{BIWA_LIBRARY_PACKAGE_ROOT_MODULE_NAME}.{BIWA_EXTENSION}"
            )));
            if lib_path.exists() && lib_path.is_file() {
                Some(lib_path)
            } else {
                None
            }
        };
        let main_path = {
            let main_path = srcpath.join(Path::new(&format!(
                "{BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME}.{BIWA_EXTENSION}"
            )));
            if main_path.exists() && main_path.is_file() {
                Some(main_path)
            } else {
                None
            }
        };

        let (pkg_kind, root_mod_id, root_path) = match (lib_path, main_path) {
            (Some(path), None) => (PackageKind::Lib, ctx.alloc_mod_id(), path),
            (None, Some(path)) => (PackageKind::Bin, ctx.alloc_mod_id(), path),
            (Some(_), Some(_)) => {
                return Err(ErrorHolder {
                    errs: vec![PkgLoadError::RootModuleDuplicated],
                    ctx: ErrorContext {
                        interner,
                        srcs,
                        metadata,
                    },
                });
            }
            (None, None) => {
                return Err(ErrorHolder {
                    errs: vec![PkgLoadError::RootModuleNotFound],
                    ctx: ErrorContext {
                        interner,
                        srcs,
                        metadata,
                    },
                });
            }
        };

        // 対応するモジュールファイルの無いディレクトリの中の `.biwa`。
        // どの `mod` 宣言からも届かないので、必ず「宣言されていないファイル」である。
        let mut orphan_files = Vec::new();

        let module_tree = ModuleTree {
            mod_id: root_mod_id,
            path: Box::new(root_path),
            mod_path: match pkg_kind {
                PackageKind::Lib => ModPath::Lib,
                PackageKind::Bin => ModPath::Main,
            },

            // root module を起点にモジュールツリーを構築する
            // それにはMainを指定する
            children: match map_module_tree_children_from_dir(
                &mut ctx,
                &srcpath,
                ModPath::Main,
                interner,
                &mut orphan_files,
            ) {
                Ok(children) => children,
                Err(e) => {
                    return Err(ErrorHolder {
                        errs: vec![e],
                        ctx: ErrorContext {
                            interner,
                            srcs,
                            metadata,
                        },
                    });
                }
            },
        };

        read_module_files(srcs, &module_tree);

        // モジュール木の形の誤り。`tolerant` なら失敗にせず返す。
        let mut structural: Vec<PkgLoadError<'static>> = orphan_files
            .into_iter()
            .map(|path| PkgLoadError::UndeclaredModuleFile {
                path: display_path(&srcpath, &path),
                declare_in: None,
            })
            .collect();

        let root_module =
            load_module::<P>(interner, srcs, module_tree, &srcpath, true, &mut structural);

        match root_module {
            Ok(root_module) if tolerant || structural.is_empty() => Ok((
                Self {
                    pkg_kind,
                    root_module,
                },
                structural,
            )),
            root_module => {
                let mut errs: Vec<PkgLoadError<'a>> = structural;
                if let Err(e) = root_module {
                    errs.extend(e);
                }
                Err(ErrorHolder {
                    errs,
                    ctx: ErrorContext {
                        interner,
                        srcs,
                        metadata,
                    },
                })
            }
        }
    }
}

/// エラーに出すファイルの位置。`src/` からの相対パスにする。
fn display_path(srcpath: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(srcpath).unwrap_or(path);
    format!("src/{}", rel.to_string_lossy())
}

fn read_module_files(srcs: &mut SourceHolder, module_tree: &ModuleTree) {
    // read children module files
    for (_, module_tree) in &module_tree.children {
        read_module_files(srcs, module_tree);
    }

    // read self module file
    let mut f = File::open(&*module_tree.path).unwrap();
    let mut src = String::new();
    f.read_to_string(&mut src).unwrap();
    srcs.mods.insert(
        module_tree.mod_id,
        ModSource {
            modu: module_tree.mod_path.clone(),
            src,
            pkg_id: PackageId::SELF_PACKAGE,
        },
    );
}

/// モジュールを構文解析し、`mod` 宣言された子モジュールを辿って読み込む。
///
/// ディレクトリにあるファイルは [`map_module_tree_children_from_dir`] がすべて拾ってある
/// (`ModId` の採番をファイル名順に保つため)。ここでは宣言と突き合わせ、
/// 宣言されたものだけを読む。
/// - 宣言されたのにファイルが無い → [`PkgLoadError::ModuleFileNotFound`]
/// - ファイルがあるのに宣言されていない → [`PkgLoadError::UndeclaredModuleFile`]
///
/// 自分の構文解析に失敗した場合は、どの子が宣言されているか分からないので子を見ない。
///
/// 構文エラーは戻り値の `Err`、モジュール木の形の誤りは `structural` に積む
/// (呼び出し側が失敗にするかを決める。[`Pkg::try_load_tolerant`])。
fn load_module<'a, P: SourceParser>(
    interner: &mut IdentInterner,
    srcs: &'a SourceHolder,
    module_tree: ModuleTree,
    srcpath: &Path,
    is_root: bool,
    structural: &mut Vec<PkgLoadError<'static>>,
) -> Result<LoadedModule, Vec<PkgLoadError<'a>>> {
    let ast = match P::parse(
        module_tree.mod_id,
        module_tree.mod_path.clone(),
        &srcs.mods.get(&module_tree.mod_id).unwrap().src,
        interner,
    ) {
        Ok(ast) => ast,
        Err(e) => {
            return Err(vec![PkgLoadError::ParseError {
                modpath: module_tree.mod_path.clone(),
                err: e,
            }]);
        }
    };

    // 宣言を集める。同じ名前の宣言が 2 つあればエラー。
    let mut declared: Vec<&ModDecl> = Vec::new();
    for g in &ast.globals {
        let Globals::Mod(decl) = g else { continue };
        if let Some(first) = declared.iter().find(|d| d.id.id == decl.id.id) {
            structural.push(PkgLoadError::DuplicatedModDecl {
                name: ident_str(interner, decl.id.id),
                first: first.span.clone(),
                second: decl.span.clone(),
            });
            continue;
        }
        declared.push(decl);
    }

    let mut files: HashMap<InternedIdent, ModuleTree> = module_tree.children.into_iter().collect();

    // 宣言の順ではなくファイル名順 (= ModId 順) に読む。
    // 識別子の intern 順などを宣言の並べ替えで変えないため。
    let mut to_load = Vec::new();
    for decl in &declared {
        let name = ident_str(interner, decl.id.id);
        match files.remove(&decl.id.id) {
            Some(child) => to_load.push((decl.id.id, child)),
            None if is_root
                && (name == BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME
                    || name == BIWA_LIBRARY_PACKAGE_ROOT_MODULE_NAME) =>
            {
                structural.push(PkgLoadError::RootModuleNameDeclared {
                    name,
                    span: decl.span.clone(),
                });
            }
            None => structural.push(PkgLoadError::ModuleFileNotFound {
                expected: display_path(
                    srcpath,
                    &module_dir(srcpath, &module_tree.mod_path)
                        .join(format!("{name}.{BIWA_EXTENSION}")),
                ),
                name,
                span: decl.span.clone(),
            }),
        }
    }

    // 残ったファイルはどこからも宣言されていない。
    let mut undeclared: Vec<ModuleTree> = files.into_values().collect();
    undeclared.sort_by_key(|m| m.mod_id);
    for child in undeclared {
        structural.push(PkgLoadError::UndeclaredModuleFile {
            path: display_path(srcpath, &child.path),
            declare_in: Some(format!("src/{}", module_tree.mod_path.file_name())),
        });
    }

    to_load.sort_by_key(|(_, m)| m.mod_id);
    let mut errs = Vec::new();
    let mut children = HashMap::new();
    for (name, child) in to_load {
        match load_module::<P>(interner, srcs, child, srcpath, false, structural) {
            Ok(module) => {
                children.insert(name, module);
            }
            Err(e) => errs.extend(e),
        }
    }

    // 自分は読めても、子モジュールが失敗していれば失敗である。
    //
    // ここで捨ててしまうと、その子モジュールが存在しなかったことになり、
    // 「定義したはずのシンボルが無い」という遠い場所のエラーだけが残る。
    if !errs.is_empty() {
        return Err(errs);
    }

    Ok(LoadedModule {
        mod_id: module_tree.mod_id,
        ast,
        children,
    })
}

fn ident_str(interner: &IdentInterner, ident: InternedIdent) -> String {
    interner
        .get_str(&ident)
        .expect("compiler bug: an identifier is not interned")
        .to_owned()
}

/// モジュール `mod_path` の子モジュールのファイルが置かれるディレクトリ。
///
/// ルートモジュールなら `src/`、`src/a/b.biwa` なら `src/a/b/`。
fn module_dir(srcpath: &Path, mod_path: &ModPath) -> PathBuf {
    match mod_path {
        ModPath::Main | ModPath::Lib => srcpath.to_path_buf(),
        ModPath::Mod(segments) => segments
            .iter()
            .fold(srcpath.to_path_buf(), |p, s| p.join(s)),
    }
}

struct ModuleTree {
    mod_id: ModId,
    mod_path: ModPath,
    path: Box<PathBuf>,
    /// 子モジュール。ファイル名順に並べる。
    ///
    /// HashMap ではなく Vec なのは、この順序が
    /// ModId の採番順・識別子の intern 順・ひいては DefId の採番順を決めるからである。
    /// 採番がビルドごとに変わると .biwameta がバイト単位で変わり、
    /// 差分ビルドの鮮度判定 (SVH) が毎回「変更あり」になってしまう。
    children: Vec<(InternedIdent, ModuleTree)>,
}

struct ModuleTreeCtx {
    next_mod_id: u32,
}

impl ModuleTreeCtx {
    fn new() -> Self {
        Self { next_mod_id: 0 }
    }

    fn alloc_mod_id(&mut self) -> ModId {
        let mod_id = ModId::new_in_self(self.next_mod_id); // usize→u64 cast is handled inside ModId::new
        self.next_mod_id += 1;
        mod_id
    }
}

// NOTE: `dir` must be directory path
// NOTE: call with ModPath::Main to load from top level directory
fn map_module_tree_children_from_dir<'a>(
    ctx: &mut ModuleTreeCtx,
    dir: &Path,
    modpath: ModPath,
    interner: &mut IdentInterner,
    orphan_files: &mut Vec<PathBuf>,
) -> Result<Vec<(InternedIdent, ModuleTree)>, PkgLoadError<'a>> {
    let mut work_dir_files: HashMap<String, (ModPath, Box<PathBuf>)> = HashMap::new();
    let mut work_dir_sub_dirs: HashMap<String, Box<PathBuf>> = HashMap::new();

    for res in dir.read_dir().unwrap_or_else(|_| {
        panic!(
            "Internal error, reading directory: {}",
            dir.to_str().unwrap()
        )
    }) {
        let entry = res.expect("Internal Error, reading directory");
        let path = entry.path();

        if path.is_dir() {
            work_dir_sub_dirs.insert(
                path.file_name().unwrap().to_str().unwrap().to_owned(),
                Box::new(path),
            );
        } else if let Some(ext) = path.extension()
            && let Some(ext_str) = ext.to_str()
            && ext_str == BIWA_EXTENSION
        {
            let file_name = path.file_stem().unwrap().to_str().unwrap().to_owned();

            // root module ではなければ登録
            // root module はトップ階層で処理
            // Mainが渡されているときは root module の意
            // matches の比較はこれで良い
            // main.biwa, lib.biwa がパッケージルートにあるが、
            // main/ や lib/ サブモジュールがあるわけではない
            if !(matches!(modpath, ModPath::Main)
                && (file_name == BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME
                    || file_name == BIWA_LIBRARY_PACKAGE_ROOT_MODULE_NAME))
            {
                let mod_path = modpath.clone().push(file_name.clone());
                work_dir_files.insert(file_name, (mod_path, Box::new(path)));
            }
        }
    }

    // モジュールと同名のディレクトリがあればサブモジュールとして再帰的にロードする
    // 各種OSのファイルシステムがファイルパスの重複を許さないことを保証する限り、
    // ここで、modulesの重複を考える必要はない
    //
    // ファイル名でソートしてから処理する。
    // read_dir の順序も HashMap の走査順も実行ごとに変わるので、
    // そのままだと ModId・intern id・DefId の採番が毎回変わってしまう。
    let mut work_dir_files: Vec<(String, (ModPath, Box<PathBuf>))> =
        work_dir_files.into_iter().collect();
    work_dir_files.sort_by(|a, b| a.0.cmp(&b.0));

    // 対応するモジュールファイルの無いディレクトリは、どのモジュールの子でもない。
    // 中の `.biwa` はどこからも宣言できないので、宣言されていないファイルとして報告する。
    let mut orphan_dirs: Vec<&Box<PathBuf>> = work_dir_sub_dirs
        .iter()
        .filter(|(name, _)| !work_dir_files.iter().any(|(f, _)| f == *name))
        .map(|(_, dir)| dir)
        .collect();
    orphan_dirs.sort();
    for dir in orphan_dirs {
        collect_biwa_files(dir, orphan_files);
    }

    work_dir_files
        .into_iter()
        .map(|(file_name, (mod_path, path))| {
            let interned = interner.get_or_insert(&file_name);
            let mod_id = ctx.alloc_mod_id();
            let children = if let Some(dir) = work_dir_sub_dirs.get(&file_name) {
                map_module_tree_children_from_dir(
                    ctx,
                    dir,
                    modpath.clone().extend(vec![file_name]),
                    interner,
                    orphan_files,
                )?
            } else {
                Vec::new()
            };

            Ok((
                interned,
                ModuleTree {
                    mod_id,
                    mod_path,
                    path,
                    children,
                },
            ))
        })
        .collect()
}

/// `dir` 以下の `.biwa` をすべて (ファイル名順に) 集める。
fn collect_biwa_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = dir.read_dir() else { return };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_biwa_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(BIWA_EXTENSION) {
            out.push(path);
        }
    }
}
