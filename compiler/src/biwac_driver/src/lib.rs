mod dep_graph;

use colored::Colorize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use biwac_base::{IdentInterner, PackageId, PackageName, SourceHolder, Target};
use biwac_dependency_metadata::{DepMetadata, ExternalPackage, SymbolIndexMap};
use biwac_fingerprint::{Fingerprint, Freshness, PackageHashes, SourceEntry, StaleReason};
use biwac_hash::Hash64;

use dep_graph::DepGraph;

/// ビルドの振る舞いの指定。
#[derive(Debug, Clone, Copy)]
pub struct BuildOptions {
    /// 鮮度判定を飛ばして全パッケージを建て直す。
    pub force_rebuild: bool,

    /// MIR までで止める。codegen は走らせない。
    ///
    /// `.biwamir` 自体は毎ビルド書かれるので、これは
    /// 「既定の出力を作らずに MIR だけ確かめたい」ときの指定である。
    pub emit_mir: bool,

    /// コード生成のターゲット。
    ///
    /// `arch` の切り落としでシンボルの集合が変わるので、
    /// `.biwameta` や `.biwamir` を含む中間生成物もすべてターゲット依存である。
    /// 成果物はターゲットごとのディレクトリに分けて置く。
    pub target: Target,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            force_rebuild: false,
            emit_mir: false,
            target: Target::TypeScript,
        }
    }
}

/// このコンパイラの同一性。
///
/// これが前回と違えばキャッシュはすべて無効になる。
/// `.biwameta` の形式版数を混ぜてあるので、形式を変えたときは
/// 「古い成果物を掴んでエラー」ではなく「フィンガープリント不一致で建て直し」になる。
fn compiler_identity() -> Hash64 {
    biwac_fingerprint::compiler_hash(&[
        biwac_dependency_metadata::BIWAC_DEPENDENCY_METADATA_FORMAT_VERSION,
        biwac_mir::BIWAC_MIR_FORMAT_VERSION,
    ])
}

/// 依存グラフの各パッケージの、今回のビルドで確定したハッシュ。
///
/// トポロジカル順 (葉から) に埋まっていくので、
/// あるパッケージを判定する時点で、その依存のハッシュは必ず揃っている。
type HashMapOfPackages = std::collections::HashMap<PackageId, PackageHashes>;

pub fn compile(pkg_root_path: PathBuf, options: BuildOptions) -> Result<(), ()> {
    println!("{}", "Compiling...".green().bold(),);

    // MIR は成果物ではなくキャッシュもされないので、
    // 鮮度判定で飛ばされると何も出力されずに終わってしまう。
    // 求められたら建て直す。
    let options = BuildOptions {
        force_rebuild: options.force_rebuild || options.emit_mir,
        ..options
    };

    let metadata = biwac_metadata_loader::try_load_package_metadata(pkg_root_path.clone())
        .map_err(|e| {
            e.print_error_message();
            biwac_base::print_error_finish_message(1);
        })?;

    println!(
        "Package: {} v{}.{}.{}",
        metadata.metadata.name.value(),
        metadata.metadata.version.major(),
        metadata.metadata.version.minor(),
        metadata.metadata.version.patch()
    );

    // 依存パッケージは <root>/.biwa_build/deps/<name>/ に取得済みである前提。
    //
    // 推移的依存も含めてここに平らに並ぶので、ビルド全体で参照する依存ディレクトリは
    // このひとつだけになる。依存パッケージ自身の deps/ は見ない。
    // (deps/greeter を建てるときも、その依存 std / color はここから引く)
    let packages_dir = biwac_base::dependencies_dir(&pkg_root_path);

    // packages_dir 自体は無くてもよい。依存が 1 つ以上あれば、
    // 取得 (`biwac_dependency_fetcher`) が必要になった時点で作られる。

    // Discover full transitive dependency graph.
    // 依存が無くても空グラフとして扱い、以降の分岐を減らす。
    let dep_graph =
        DepGraph::discover(&metadata.metadata.dependencies, &packages_dir).map_err(|_| {
            biwac_base::print_error_finish_message(1);
        })?;

    // Build in topological order (leaves = no deps first).
    // Items within the same batch are independent and can be parallelized (future tokio).
    let batches = dep_graph.topo_batches().map_err(|_| {
        biwac_base::print_error_finish_message(1);
    })?;

    let mut svhs = HashMapOfPackages::new();
    for batch in &batches {
        // TODO: parallelize within batch using tokio

        for dep_name in batch {
            let dep_root = packages_dir.join(dep_name);
            let svh = build_or_reuse_package(
                dep_root,
                &packages_dir,
                &dep_graph,
                &svhs,
                options,
                DisplayDepth::Dependency,
            )?;
            let pkg_id = dep_graph
                .pkg_id(dep_name)
                .expect("package must be in graph");
            svhs.insert(pkg_id, svh);
        }
    }

    // 生成物は import 文が `./<package>.ts` を指すため、
    // 自パッケージと推移的依存の .ts が同じディレクトリに並んでいる必要がある。
    //
    // 再ビルドしたものだけでなく **グラフの全パッケージ** を対象にする。
    // キャッシュが効いた依存の .ts も要るし、
    // 依存から外れたパッケージの .ts は消さなければならない。
    let build_dir_path = prepare_build_dir(&pkg_root_path, options.target)?;
    if !options.emit_mir {
        collect_dep_bins(
            &dep_graph.all_packages(),
            metadata.metadata.name.value(),
            &packages_dir,
            &build_dir_path,
            options.target,
        )?;
    }

    build_or_reuse_package(
        pkg_root_path,
        &packages_dir,
        &dep_graph,
        &svhs,
        options,
        DisplayDepth::Root,
    )?;

    println!("{}", "Finished!".green().bold(),);

    Ok(())
}

/// 進捗表示のインデント。ルートパッケージと依存で見た目を変えるだけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayDepth {
    Root,
    Dependency,
}

impl DisplayDepth {
    fn indent(&self) -> &'static str {
        match self {
            Self::Root => "",
            Self::Dependency => "  ",
        }
    }
}

/// パッケージ 1 つを、必要なら再ビルドする。
///
/// 戻り値はそのパッケージの SVH (インタフェースのハッシュ)。
/// キャッシュを使った場合は前回の値をそのまま返す。
/// 呼び出し側はこれを下流のパッケージの鮮度判定に渡す。
fn build_or_reuse_package(
    pkg_root: PathBuf,
    packages_dir: &Path,
    dep_graph: &DepGraph,
    svhs: &HashMapOfPackages,
    options: BuildOptions,
    depth: DisplayDepth,
) -> Result<PackageHashes, ()> {
    let metadata =
        biwac_metadata_loader::try_load_package_metadata(pkg_root.clone()).map_err(|e| {
            e.print_error_message();
            biwac_base::print_error_finish_message(1);
        })?;
    let pkg_name = metadata.metadata.name.value().to_string();

    let build_dir_path = prepare_build_dir(&pkg_root, options.target)?;

    // このパッケージのビルドが読むことになる依存の集合 = 推移閉包。
    //
    // ルートパッケージはグラフに含まれないので、
    // その推移閉包はグラフ全体そのものになる。
    let transitive_deps = match depth {
        DisplayDepth::Root => dep_graph.all_packages(),
        DisplayDepth::Dependency => dep_graph.transitive_deps(&pkg_name),
    };
    let dep_hashes: Vec<(PackageId, PackageHashes)> = transitive_deps
        .iter()
        .filter_map(|name| {
            let pkg_id = dep_graph.pkg_id(name)?;
            Some((pkg_id, *svhs.get(&pkg_id)?))
        })
        .collect();

    let sources = biwac_fingerprint::collect_sources(&pkg_root).map_err(|e| {
        eprintln!("Error: failed to read sources of `{pkg_name}`: {e}");
        biwac_base::print_error_finish_message(1);
    })?;

    // wasm では単相化がプログラム全体の操作なので、
    // 根を持たないライブラリからは成果物が出ない。
    let produces_binary =
        is_playable_package(&pkg_root) || options.target.library_produces_binary();

    let freshness = check_freshness(
        &build_dir_path,
        &pkg_name,
        &metadata.metadata,
        &dep_hashes,
        &sources,
        options,
        produces_binary,
    );

    if let Freshness::Fresh(hashes) = freshness {
        println!(
            "{}{} {} v{}.{}.{}",
            depth.indent(),
            "Fresh".cyan().bold(),
            pkg_name,
            metadata.metadata.version.major(),
            metadata.metadata.version.minor(),
            metadata.metadata.version.patch(),
        );
        return Ok(hashes);
    }
    let Freshness::Stale(reason) = freshness else {
        unreachable!()
    };

    println!(
        "{}{} {} v{}.{}.{} ({})",
        depth.indent(),
        "Compiling".green().bold(),
        pkg_name,
        metadata.metadata.version.major(),
        metadata.metadata.version.minor(),
        metadata.metadata.version.patch(),
        reason.describe(),
    );

    let mut interner = IdentInterner::new();
    let direct_dep_names: Vec<String> = metadata
        .metadata
        .dependencies
        .iter()
        .map(|d| d.name.value().to_string())
        .collect();

    let external_packages = load_external_packages(
        &transitive_deps,
        &direct_dep_names,
        dep_graph,
        packages_dir,
        options.target,
        &mut interner,
    )?;

    // 依存の `.biwamir` を読む。
    //
    // 単相化の入力であり、同時に「書き出した MIR が読み戻せるか」の検査でもある。
    // 依存の本体を取り込むターゲット (wasm) では必須で、
    // そうでないターゲットでも `--emit mir` のときは検査のために読む。
    let dep_mirs = if options.emit_mir || options.target.consumes_dependency_mir() {
        let deps: Vec<(PackageId, String, Hash64)> = external_packages
            .iter()
            .filter_map(|p| {
                Some((
                    p.pkg_id,
                    interner.get_str(&p.ident)?.to_string(),
                    p.meta.svh,
                ))
            })
            .collect();
        load_dep_mirs(&deps, packages_dir, options.target, &mut interner, depth)?
    } else {
        Vec::new()
    };

    let hashes = load_analyze_and_codegen_single_package(
        external_packages,
        &mut interner,
        &metadata,
        &dep_hashes,
        dep_mirs,
        pkg_root,
        build_dir_path.clone(),
        options,
    )?;

    // `--emit` を付けたビルドは既定の生成物を作っていないので、
    // 鮮度を記録してはいけない。記録すると次回「Fresh」と言い張って
    // 生成物が無いまま成功してしまう。
    if options.emit_mir {
        return Ok(hashes);
    }

    // 次回の鮮度判定のために、今回のビルドの状態を記録する。
    let fingerprint = Fingerprint::of_build(
        compiler_identity(),
        &metadata.metadata,
        hashes,
        &dep_hashes,
        sources,
    );
    let fp_path = biwac_fingerprint::fingerprint_path(
        &target_dir(&build_dir_path, options.target),
        &pkg_name,
    );
    std::fs::write(&fp_path, fingerprint.encode_file()).map_err(|e| {
        eprintln!("Error: failed to write {:?}: {}", fp_path, e);
        biwac_base::print_error_finish_message(1);
    })?;

    Ok(hashes)
}

/// 前回のビルドから状況が変わっていないかを判定する。
///
/// 判定材料は [`biwac_fingerprint`] に閉じている。
/// ここは「前回の記録を読み出す」ところだけを持つ。
fn check_freshness(
    build_dir_path: &Path,
    pkg_name: &str,
    metadata: &biwac_base::PackageMetadata,
    dep_hashes: &[(PackageId, PackageHashes)],
    sources: &[SourceEntry],
    options: BuildOptions,
    // このターゲットでこのパッケージが成果物を持つか。
    produces_binary: bool,
) -> Freshness {
    if options.force_rebuild {
        return Freshness::Stale(StaleReason::Forced);
    }

    // シグニチャと本体のキャッシュが無ければ、記録があっても意味がない。
    let target = options.target;
    if !metadata_path(build_dir_path, target, pkg_name).exists()
        || !mir_path(build_dir_path, target, pkg_name).exists()
    {
        return Freshness::Stale(StaleReason::NoPreviousBuild);
    }

    // 生成物が消えていれば、記録がどうであれ建て直す。
    // 出力ディレクトリだけ消したときに「Fresh」と言い張って、
    // 生成物が無いまま成功してしまうのを防ぐ。
    //
    // ただし、そのターゲットでそのパッケージが成果物を持つとは限らない。
    // wasm は単相化を通すので、根を持たないライブラリからは何も出ない。
    if produces_binary && !bin_path(build_dir_path, target, pkg_name).exists() {
        return Freshness::Stale(StaleReason::MissingOutput);
    }

    let fp_path =
        biwac_fingerprint::fingerprint_path(&target_dir(build_dir_path, target), pkg_name);
    let Ok(data) = std::fs::read(&fp_path) else {
        return Freshness::Stale(StaleReason::NoPreviousBuild);
    };
    // 形式が変わった / 壊れている場合は、単に「前回の情報は使えない」と扱って建て直す。
    let Ok(previous) = Fingerprint::decode_file(&data) else {
        return Freshness::Stale(StaleReason::UnreadableFingerprint);
    };

    previous.freshness(
        compiler_identity(),
        metadata,
        dep_hashes,
        sources,
        options.target.consumes_dependency_mir(),
    )
}

/// playable (`main.biwa` を持つ) パッケージか。
///
/// パッケージを読み込む前に知りたいので、ルートモジュールの有無で判定する。
/// 判定規則は [`biwac_package_loader`] のものと同じである。
fn is_playable_package(pkg_root: &Path) -> bool {
    pkg_root
        .join("src")
        .join(format!(
            "{}.{}",
            biwac_base::BIWA_BINARY_PACKAGE_ROOT_MODULE_NAME,
            biwac_base::BIWA_EXTENSION
        ))
        .is_file()
}

/// ターゲットごとの中間生成物と成果物を入れるディレクトリ。
///
/// `arch` の切り落としでシンボルの集合がターゲットごとに変わるので、
/// `.biwameta` も `.biwamir` もターゲット依存である。
/// ここで分けておけば、ターゲットを切り替えても互いのキャッシュを壊さない。
fn target_dir(build_dir_path: &Path, target: Target) -> PathBuf {
    build_dir_path.join(target.build_subdir())
}

fn metadata_path(build_dir_path: &Path, target: Target, pkg_name: &str) -> PathBuf {
    target_dir(build_dir_path, target).join(format!("{pkg_name}.biwameta"))
}

/// MIR のキャッシュ。`.biwameta` と対で置かれる。
fn mir_path(build_dir_path: &Path, target: Target, pkg_name: &str) -> PathBuf {
    target_dir(build_dir_path, target).join(format!("{pkg_name}.{}", biwac_mir::MIR_FILE_EXTENSION))
}

/// codegen の出力先。
fn bin_path(build_dir_path: &Path, target: Target, pkg_name: &str) -> PathBuf {
    target_dir(build_dir_path, target).join(format!("{pkg_name}.{}", target.bin_extension()))
}

fn prepare_build_dir(pkg_root: &Path, target: Target) -> Result<PathBuf, ()> {
    let build_dir_path = pkg_root.join(Path::new(biwac_base::BIWA_BUILD_DIRECTORY_NAME));
    if build_dir_path.exists() && !build_dir_path.is_dir() {
        panic!(
            "Destination directory broken, conflicted file found: `{}`",
            build_dir_path
                .as_os_str()
                .to_str()
                .expect("broken build directory path")
        );
    }
    let dir = target_dir(&build_dir_path, target);
    std::fs::create_dir_all(&dir).map_err(|e| {
        eprintln!("Error: failed to create {:?}: {}", dir, e);
        biwac_base::print_error_finish_message(1);
    })?;
    Ok(build_dir_path)
}

/// 依存グラフの推移閉包すべての `.biwameta` をロードする。
///
/// 直接依存だけでは足りない。依存の `.biwameta` に載っているシグニチャが
/// さらにその依存の型を参照していることがあり
/// (`greeter::theme() -> color::Rgb`)、
/// その参照を DefId に復元するには相手のメタデータが要るからである。
/// 名前で引ける (= import できる) のは直接依存だけなので、
/// [`ExternalPackage::direct`] で区別する。
fn load_external_packages(
    transitive_deps: &[String],
    direct_dep_names: &[String],
    dep_graph: &DepGraph,
    packages_dir: &Path,
    target: Target,
    interner: &mut IdentInterner,
) -> Result<Vec<ExternalPackage>, ()> {
    let mut packages = Vec::with_capacity(transitive_deps.len());
    for name in transitive_deps {
        let Some(pkg_id) = dep_graph.pkg_id(name) else {
            continue;
        };
        let dep_root = packages_dir.join(name);
        let meta = load_dep_metadata(&dep_root, target, name)?;
        packages.push(ExternalPackage {
            ident: interner.get_or_insert(name),
            pkg_id,
            meta: Arc::new(meta),
            direct: direct_dep_names.iter().any(|d| d == name),
        });
    }

    Ok(packages)
}

/// Loads a .biwameta file from a built dependency's build directory.
fn load_dep_metadata(dep_root: &Path, target: Target, dep_name: &str) -> Result<DepMetadata, ()> {
    let meta_path = dep_root
        .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
        .join(target.build_subdir())
        .join(format!("{}.biwameta", dep_name));
    let data = std::fs::read(&meta_path).map_err(|e| {
        eprintln!("Error: failed to read {:?}: {}", meta_path, e);
    })?;
    DepMetadata::decode_file(&data).map_err(|e| {
        eprintln!("Error: failed to decode {:?}: {}", meta_path, e);
    })
}

/// Persist self package's symbol metadata to disk for dependents.
///
/// 生成した `.biwameta` の SVH と、そこで決まったシンボルの採番を返す。
/// 採番は `.biwamir` を書くときにそのまま使う
/// (下流から見たこのパッケージの DefId はこの採番で決まる)。
fn persist_dep_metadata(
    hir: &biwac_hir::Hir,
    lang_items: &biwac_lang_item::LangItemTable,
    host_exports: &biwac_host_export::HostExportTable,
    srcs: &biwac_base::SourceHolder,
    interner: &biwac_base::IdentInterner,
    dep_hashes: &[(PackageId, PackageHashes)],
    build_dir_path: &Path,
    target: Target,
    metadata: &biwac_base::MetadataHolder,
) -> Result<(Hash64, SymbolIndexMap), ()> {
    // `.biwameta` が記録するのはインタフェースの伝播に使う SVH だけである。
    let dep_svhs: Vec<(PackageId, Hash64)> =
        dep_hashes.iter().map(|(id, h)| (*id, h.svh)).collect();
    let (dep_meta, symbol_index) =
        DepMetadata::new(hir, srcs, interner, lang_items, host_exports, &dep_svhs);
    let svh = dep_meta.svh;
    let meta_bytes = dep_meta.encode_file();
    let meta_path = metadata_path(build_dir_path, target, metadata.metadata.name.value());
    std::fs::write(&meta_path, meta_bytes)
        .map_err(|e| {
            eprintln!("Error: failed to write {:?}: {}", meta_path, e);
        })
        .map(|_| (svh, symbol_index))
}

/// パイプライン本体。生成した `.biwameta` と `.biwamir` のハッシュを返す。
fn load_analyze_and_codegen_single_package(
    external_packages: Vec<ExternalPackage>,
    interner: &mut biwac_base::IdentInterner,
    metadata: &biwac_base::MetadataHolder,
    dep_hashes: &[(PackageId, PackageHashes)],
    dep_mirs: Vec<(PackageId, biwac_mir::Mir)>,
    pkg_root_path: PathBuf,
    build_dir_path: PathBuf,
    options: BuildOptions,
) -> Result<PackageHashes, ()> {
    // 型推論と codegen は「名前で引けるか」を問わないので、
    // direct かどうかを落として推移閉包すべてを渡す。
    let ext_pkgs_for_ty: Vec<(biwac_base::PackageId, Arc<DepMetadata>)> = external_packages
        .iter()
        .map(|p| (p.pkg_id, Arc::clone(&p.meta)))
        .collect();

    let mut srcs = SourceHolder::default();
    let package_name_interned = interner.get_or_insert(metadata.metadata.name.value());

    let mut pkg = biwac_package_loader::Pkg::try_load::<biwac_package_loader::BiwacSourceParser>(
        metadata,
        interner,
        &mut srcs,
        pkg_root_path,
    )
    .map_err(|e| e.print_error_messages())?;

    // Attribute check: AST から HIR への lowering の前に、
    // 既知の属性か / キー・値型 / 付与対象を検証する。
    // 後段の lang item 回収はこれを通過していることを前提にできる。
    check_attributes(&pkg, interner, &srcs, metadata)?;

    // 選択されていない arch の native をここで落とす。
    //
    // std は同じ名前で arch 違いの native を並べるので、
    // このまま名前解決に渡すとシンボルが衝突する。
    // def collection より前に刈り込んでおけば、
    // 以降のパスは「残っているのは選択された arch のものだけ」を前提にできる。
    {
        let target = options.target;
        pkg.walk_modules_mut(|module| {
            biwac_attribute::retain_for_target(&mut module.ast, target, interner);
        });
    }

    let pkg_kind = pkg.pkg_kind;
    let root_mod_id = pkg.root_module.mod_id;

    let biwac_name_resolver::ResolveOutput {
        hir,
        lang_items,
        host_exports,
    } = biwac_name_resolver::NameResolver::new(
        metadata,
        external_packages,
        package_name_interned,
        &mut pkg,
    )
    .unwrap()
    .try_resolve(interner)
    .map_err(|errs| print_errors(&errs, interner, &srcs, metadata))?;

    // scene のシグネチャ (`(Game[..]) -> Game[..]`) と、
    // playable package のエントリポイント (`fn main()`) を検証する。
    // シグネチャは名前解決の時点で確定しているので型推論より前に走らせる。
    // 2 つの検査は独立しているので、両方の誤りをまとめて報告する。
    let scene_errors = biwac_scene::check(&hir, &lang_items, interner)
        .err()
        .unwrap_or_default();
    let entrypoints = biwac_entrypoint::check(&hir, pkg_kind, root_mod_id, interner);
    let entrypoint_errors = entrypoints.as_ref().err().map_or(&[][..], Vec::as_slice);
    let errors: Vec<&dyn biwac_base::BiwacError> = (scene_errors.iter().map(|e| e as _))
        .chain(entrypoint_errors.iter().map(|e| e as _))
        .collect();
    if !errors.is_empty() {
        print_errors(&errors, interner, &srcs, metadata);
        return Err(());
    }
    let entrypoints = entrypoints.unwrap_or_default();

    let hir =
        biwac_type_inferrer::TyCtx::new(hir, lang_items.clone(), ext_pkgs_for_ty.clone(), interner)
            .infer()
            .map_err(|e| {
                print_errors(std::slice::from_ref(e.as_ref()), interner, &srcs, metadata)
            })?;

    // Persist self package's symbol metadata to disk for dependents.
    // lang item テーブルと host export のフラグも書き出すので、
    // 依存側はこれを読んで復元する。
    //
    // 型推論の後に書く。無名関数を持ち上げた関数は型推論の後でできるが、
    // 依存元の単相化が `.biwamir` 越しに参照するのでシンボルが要る
    // (型推論はシグニチャを変えないので、それ以外の中身は推論の前と同じである)。
    let (svh, symbol_index) = persist_dep_metadata(
        &hir,
        &lang_items,
        &host_exports,
        &srcs,
        interner,
        dep_hashes,
        &build_dir_path,
        options.target,
        metadata,
    )?;

    // MIR は `.biwameta` と対で毎ビルド書き出す。
    // 単相化するターゲットは依存パッケージの本体を必要とするので、
    // 「そのターゲットのときだけ書く」形にはできない
    // (ある日 wasm を建てようとしたら依存の MIR が無い、ということになる)。
    let (mir_hash, mir) = persist_mir(
        &hir,
        interner,
        &symbol_index,
        svh,
        &build_dir_path,
        options.target,
        metadata,
    )?;
    let hashes = PackageHashes { svh, mir: mir_hash };

    // `--emit` は既定の出力を置き換える (rustc と同じ流儀)。
    // 中間表現だけを見たいときに codegen まで走らせる理由が無いのと、
    // 中間表現の検証をターゲットの実装状況に縛られずに行えるようにするため。
    if options.emit_mir {
        // 単相化できるのは根を持つパッケージ、つまり playable なものだけである。
        // ライブラリはどの型で実体化されるかを知らないので、
        // ジェネリックなままの `.biwamir` を出して終わる。
        if pkg_kind.is_playable() {
            let mono = monomorphize_program(
                &hir,
                &mir,
                &ext_pkgs_for_ty,
                &dep_mirs,
                &entrypoints,
                &host_exports,
                interner,
            )?;
            println!(
                "{} {} instance(s), {} type(s)",
                "Monomorphized".cyan().bold(),
                mono.instances.len(),
                mono.types.len(),
            );
            last_monomorphized(mono);
        }
        return Ok(hashes);
    }

    match options.target {
        Target::TypeScript => {
            // TypeScript は tier 2 で、ジェネリクスの trait 制限に未対応である。
            //
            // ジェネリクスを保ったまま 1 回だけ出力する設計なので、
            // 実行時に型引数が残らず、`T::guee()` の呼び先を決められない。
            // 対応するには witness (辞書) を引数で渡す仕組みが要る
            // (`docs/trait.md` の第 3 段)。
            //
            // 黙って壊れたものを出さないよう、codegen の手前で止める。
            // MIR は TypeScript でも組み立てているので、ここで見られる。
            let unsupported: Vec<TraitBoundOnTypeScript> = trait_calls_in(&mir)
                .into_iter()
                .map(|span| TraitBoundOnTypeScript { span })
                .collect();
            if !unsupported.is_empty() {
                print_errors(&unsupported, interner, &srcs, metadata);
                return Err(());
            }

            // scene の値も TypeScript では扱えない。
            // scene は generator function で、呼ぶ側が `yield*` で委譲しなければならないが、
            // 関数の値を通した呼び出しでは、呼び先が scene かどうかが呼ぶ側で分からない。
            let is_scene = |def_id: &biwac_span::ValDefId| {
                if def_id.pkg().is_self() {
                    matches!(
                        hir.vals.get(def_id),
                        Some(biwac_hir::ValDefKind::NovelScene(_))
                    )
                } else {
                    ext_pkgs_for_ty
                        .iter()
                        .find(|(pkg_id, _)| *pkg_id == def_id.pkg())
                        .is_some_and(|(_, meta)| meta.is_scene(def_id.local_idx()))
                }
            };
            let unsupported: Vec<SceneValueOnTypeScript> = scene_values_in(&mir, is_scene)
                .into_iter()
                .map(|span| SceneValueOnTypeScript { span })
                .collect();
            if !unsupported.is_empty() {
                print_errors(&unsupported, interner, &srcs, metadata);
                return Err(());
            }

            // codegen も lang item を使う。
            // novel statement を std の関数呼び出しに展開するため。
            // 外部パッケージのメタデータはシンボル名のマングリングに使う
            // (外部シンボルは HIR に無く span もダミーのため)。
            let bin = biwac_generator::arch::typescript::generate(
                &hir,
                interner,
                &srcs,
                &ext_pkgs_for_ty,
                &entrypoints,
                &host_exports,
            );

            write_bin(
                &build_dir_path,
                options.target,
                &metadata.metadata.name,
                &bin,
            )
            .unwrap();
        }
        Target::Wasm => {
            // ライブラリは wasm の成果物を持たない。
            // 単相化はプログラム全体の操作で、根を持つのは playable だけである。
            if pkg_kind.is_playable() {
                let mono = monomorphize_program(
                    &hir,
                    &mir,
                    &ext_pkgs_for_ty,
                    &dep_mirs,
                    &entrypoints,
                    &host_exports,
                    interner,
                )?;

                // マングリングは TypeScript と同じものを使う。
                // ホスト側の実装がターゲット間で対応付けやすくなる。
                let mangler =
                    biwac_generator::mangle::Mangler::new(&hir, interner, &srcs, &ext_pkgs_for_ty);

                let wat = biwac_generator::arch::wasm::emit(&mono, &mangler, &host_exports)
                    .map_err(|e| {
                        eprintln!("Error: wasm code generation failed: {e}");
                        biwac_base::print_error_finish_message(1);
                    })?;

                // .wat は成果物として残す。デバッグではこちらを読む。
                let wat_path = target_dir(&build_dir_path, options.target)
                    .join(format!("{}.wat", metadata.metadata.name.value()));
                std::fs::write(&wat_path, &wat).map_err(|e| {
                    eprintln!("Error: failed to write {:?}: {}", wat_path, e);
                    biwac_base::print_error_finish_message(1);
                })?;

                // アセンブルと検証はその場で行う。外部ツールは要らない。
                let binary = biwac_generator::arch::wasm::assemble(&wat).map_err(|e| {
                    eprintln!("Error: {e}");
                    eprintln!("  (see {})", wat_path.display());
                    biwac_base::print_error_finish_message(1);
                })?;

                let path = bin_path(
                    &build_dir_path,
                    options.target,
                    metadata.metadata.name.value(),
                );
                std::fs::write(&path, binary).map_err(|e| {
                    eprintln!("Error: failed to write {:?}: {}", path, e);
                    biwac_base::print_error_finish_message(1);
                })?;
            }
        }
    }

    Ok(hashes)
}

/// MIR を構築して `.biwamir` に書き出し、そのハッシュを返す。
///
/// 不変条件の検査もここで走らせる。
/// 検査に落ちるのはコンパイラのバグなので、黙って出力せずエラーにする。
///
/// シンボルは `symbol_index` で `.biwameta` の索引に読み替えて書く。
/// 下流から見たこのパッケージのシンボルの DefId はその索引で決まるので、
/// `.biwamir` も同じ空間で書かなければ噛み合わない。
fn persist_mir(
    hir: &biwac_hir::Hir,
    interner: &biwac_base::IdentInterner,
    symbol_index: &SymbolIndexMap,
    meta_svh: Hash64,
    build_dir_path: &Path,
    target: Target,
    metadata: &biwac_base::MetadataHolder,
) -> Result<(Hash64, biwac_mir::Mir), ()> {
    let pkg_id = self_package_id(&metadata.metadata);
    let mut mir = biwac_mir_build::build(hir, pkg_id);

    let errors = biwac_mir::validate(&mir);
    if !errors.is_empty() {
        eprintln!("Error: built MIR is broken (this is a compiler bug):");
        for e in &errors {
            eprintln!("  {e}");
        }
        biwac_base::print_error_finish_message(errors.len());
        return Err(());
    }

    // 書き出す前に畳む。`.biwamir` に載る形も、下流が単相化に使う形も
    // これを通したあとのものになる。
    biwac_mir_transform::run_passes(&mut mir, biwac_mir_transform::default_passes()).map_err(
        |e| {
            eprintln!("Error: {e}");
            biwac_base::print_error_finish_message(1);
        },
    )?;

    let text = biwac_mir::encode(
        &mir,
        &biwac_mir::EncodeCtx {
            pkg_id,
            meta_svh,
            symbols: Some(symbol_index),
            interner,
        },
    );

    let path = mir_path(build_dir_path, target, metadata.metadata.name.value());
    std::fs::write(&path, &text).map_err(|e| {
        eprintln!("Error: failed to write {:?}: {}", path, e);
        biwac_base::print_error_finish_message(1);
    })?;

    Ok((mir_hash(&text), mir))
}

/// 直近の単相化の結果を、テストから覗けるようにしておく。
///
/// 結果はファイルにしないので、テストは in-process でこれを見る。
/// 本番の経路では書くだけで、誰も読まない。
#[cfg(test)]
fn last_monomorphized(mono: biwac_mir::MonoMir) {
    tests::LAST_MONO.with(|slot| *slot.borrow_mut() = Some(mono));
}

#[cfg(not(test))]
fn last_monomorphized(_mono: biwac_mir::MonoMir) {}

/// プログラム全体を単相化する。
///
/// 単相化の結果はファイルにしない。消費者は次に入るバックエンドで、
/// それはメモリ上で受け取れば足りるためである。
/// ここでは「通ること」と規模だけを確かめる。
fn monomorphize_program(
    hir: &biwac_hir::Hir,
    own: &biwac_mir::Mir,
    ext_pkgs: &[(PackageId, Arc<DepMetadata>)],
    dep_mirs: &[(PackageId, biwac_mir::Mir)],
    entrypoints: &biwac_entrypoint::Entrypoints,
    host_exports: &biwac_host_export::HostExportTable,
    interner: &mut IdentInterner,
) -> Result<biwac_mir::MonoMir, ()> {
    // 根はランタイムが名前で呼ぶ `fn main()` と、host export された関数である。
    // そこから辿れない関数は成果物に入らない (到達性による除去がここで効く)。
    // host export された関数はどこからも呼ばれていなくても
    // ホストから直接呼ばれうるので、除去されては困る。
    // 依存パッケージ (std 等) が host export した関数もここに含まれる
    // (名前解決が `.biwameta` から取り込んでいる)。
    let roots: Vec<biwac_span::ValDefId> = biwac_entrypoint::Entrypoint::ALL
        .iter()
        .filter_map(|s| entrypoints.get(*s))
        .chain(host_exports.iter().map(|(def_id, _)| def_id))
        .collect();

    let deps: Vec<(PackageId, &DepMetadata, &biwac_mir::Mir)> = dep_mirs
        .iter()
        .filter_map(|(pkg_id, mir)| {
            let meta = ext_pkgs.iter().find(|(id, _)| id == pkg_id)?;
            Some((*pkg_id, meta.1.as_ref(), mir))
        })
        .collect();

    biwac_mir_transform::monomorphize(biwac_mir_transform::MonoInput {
        hir,
        own,
        deps: &deps,
        roots: &roots,
        interner,
    })
    .map_err(|errors| {
        eprintln!("Error: monomorphization failed:");
        for e in &errors {
            eprintln!("  {e}");
        }
        biwac_base::print_error_finish_message(errors.len());
    })
}

/// `.biwamir` の内容のハッシュ。
///
/// 依存の本体が変わったかどうかの判定に使う。
/// エンコードは決定論的なので、同じ MIR なら同じ値になる。
fn mir_hash(text: &str) -> Hash64 {
    use biwac_hash::StableHasher64;
    let mut h = StableHasher64::new();
    h.write_str(text);
    h.finish()
}

/// このパッケージの [`PackageId`]。
///
/// メモリ上では自パッケージのシンボルは `SELF` を持つが、
/// ディスクに書くときは他のパッケージと同じ土俵に載せる必要がある。
/// 依存側が振るのと同じ規則 (`(name, version)` のハッシュ) で導出する。
fn self_package_id(metadata: &biwac_base::PackageMetadata) -> PackageId {
    biwac_span::PackageHashId::new(&metadata.name, &metadata.version).as_package_id()
}

/// 依存の `.biwamir` をすべて読む。
fn load_dep_mirs(
    deps: &[(PackageId, String, Hash64)],
    packages_dir: &Path,
    target: Target,
    interner: &mut IdentInterner,
    depth: DisplayDepth,
) -> Result<Vec<(PackageId, biwac_mir::Mir)>, ()> {
    if deps.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(deps.len());
    let mut items = 0;
    for (pkg_id, name, svh) in deps {
        let root = packages_dir.join(name);
        let mir = load_dep_mir(&root, target, name, *svh, interner).map_err(|_| {
            biwac_base::print_error_finish_message(1);
        })?;
        items += mir.items.len();
        out.push((*pkg_id, mir));
    }
    println!(
        "{}{} {} dependency MIR file(s), {} item(s)",
        depth.indent(),
        "Loaded".cyan().bold(),
        deps.len(),
        items,
    );
    Ok(out)
}

/// 依存パッケージの `.biwamir` を読む。
///
/// `.biwameta` と対で書かれているので、対応が崩れていないかをここで確かめる。
/// `.biwamir` はシンボルを `.biwameta` の索引で参照しており、
/// 索引がずれれば SVH も変わるので、SVH の照合で誤読を止められる
/// (rustc が `CrateDep { name, hash: Svh }` でやっているのと同じ)。
fn load_dep_mir(
    dep_root: &Path,
    target: Target,
    dep_name: &str,
    expected_meta_svh: Hash64,
    interner: &mut IdentInterner,
) -> Result<biwac_mir::Mir, ()> {
    let path = dep_root
        .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
        .join(target.build_subdir())
        .join(format!("{dep_name}.{}", biwac_mir::MIR_FILE_EXTENSION));

    let text = std::fs::read_to_string(&path).map_err(|e| {
        eprintln!("Error: failed to read {:?}: {}", path, e);
    })?;

    let decoded = biwac_mir::decode(&text, interner).map_err(|e| {
        eprintln!("Error: failed to decode {:?}: {}", path, e);
    })?;

    if decoded.meta_svh != expected_meta_svh {
        eprintln!(
            "Error: {:?} was built against a different `{dep_name}.biwameta` \
             (recorded {}, found {})",
            path, decoded.meta_svh, expected_meta_svh
        );
        return Err(());
    }

    Ok(decoded.mir)
}

/// パッケージ内の全モジュールに属性検証パスを走らせる。
///
/// モジュール木の走査はここが持つ。
/// biwac_attribute は biwac_ast までしか知らない
/// (biwac_package_loader は biwac_parser に依存しており、
///  そこに依存させると循環する)。
fn check_attributes(
    pkg: &biwac_package_loader::Pkg,
    interner: &biwac_base::IdentInterner,
    srcs: &biwac_base::SourceHolder,
    metadata: &biwac_base::MetadataHolder,
) -> Result<(), ()> {
    let mut errors = Vec::new();
    pkg.walk_modules(|module| {
        biwac_attribute::check_mod_ast(&module.ast, interner, &mut errors);
    });

    if errors.is_empty() {
        return Ok(());
    }

    print_errors(&errors, interner, srcs, metadata);

    Err(())
}

/// 収集済みのエラーをまとめて表示する。
/// TypeScript ターゲットが扱えない、実装が単相化まで決まらない呼び出し。
#[derive(Debug)]
struct TraitBoundOnTypeScript {
    span: biwac_span::Span,
}

impl biwac_base::BiwacError for TraitBoundOnTypeScript {
    fn print_error_message(&self, ctx: &biwac_base::ErrorContext) {
        ctx.diagnostic("The TypeScript target does not support trait bounds on generics yet.")
            .label(
                biwac_base::DiagSpan::new(self.span.module(), self.span.begin(), self.span.end()),
                "the implementation cannot be chosen here",
            )
            .note(
                "TypeScript keeps generics instead of monomorphizing, \
                 so the type argument is gone at run time; \
                 build for the wasm target, or avoid the bound",
            )
            .print();
    }
}

/// TypeScript ターゲットが扱えない、scene の値。
#[derive(Debug)]
struct SceneValueOnTypeScript {
    span: biwac_span::Span,
}

impl biwac_base::BiwacError for SceneValueOnTypeScript {
    fn print_error_message(&self, ctx: &biwac_base::ErrorContext) {
        ctx.diagnostic("The TypeScript target does not support scenes as values yet.")
            .label(
                biwac_base::DiagSpan::new(self.span.module(), self.span.begin(), self.span.end()),
                "a scene is used as a value here",
            )
            .note(
                "a scene is a generator function in TypeScript and must be delegated to with \
                 `yield*`, but a call through a function value cannot tell whether it calls a scene; \
                 build for the wasm target",
            )
            .print();
    }
}

/// scene を値として使っている位置 (scene への関数参照) を集める。
fn scene_values_in(
    mir: &biwac_mir::Mir,
    is_scene: impl Fn(&biwac_span::ValDefId) -> bool,
) -> Vec<biwac_span::Span> {
    use biwac_mir::{Const, Operand, Rvalue, StatementKind, TerminatorKind};

    let refers_scene =
        |op: &Operand| matches!(op, Operand::Const(Const::FnDef(def_id, _)) if is_scene(def_id));

    let mut out = Vec::new();
    for item in mir.items.values() {
        let biwac_mir::MirItem::Body(body) = item else {
            continue;
        };
        for block in &body.blocks {
            for stmt in &block.stmts {
                let StatementKind::Assign(_, rvalue) = &stmt.kind;
                let found = match rvalue {
                    Rvalue::Use(op) | Rvalue::UnaryOp(_, op) => refers_scene(op),
                    Rvalue::BinaryOp(_, l, r) => refers_scene(l) || refers_scene(r),
                    Rvalue::Aggregate(_, members) => members.iter().any(|(_, op)| refers_scene(op)),
                    Rvalue::Discriminant(_) => false,
                };
                if found {
                    out.push(stmt.span.clone());
                }
            }
            if let TerminatorKind::Call { callee, args, .. } = &block.term.kind {
                let callee_is_scene =
                    matches!(callee, biwac_mir::Callee::Indirect(op) if refers_scene(op));
                if callee_is_scene || args.iter().any(refers_scene) {
                    out.push(block.term.span.clone());
                }
            }
        }
    }
    out.sort_by_key(|s| s.begin());
    out
}

/// 実装がまだ決まっていない呼び出しの位置を集める。
fn trait_calls_in(mir: &biwac_mir::Mir) -> Vec<biwac_span::Span> {
    let mut out = Vec::new();
    for item in mir.items.values() {
        let biwac_mir::MirItem::Body(body) = item else {
            continue;
        };
        for block in &body.blocks {
            if let biwac_mir::TerminatorKind::Call {
                callee: biwac_mir::Callee::TraitAssoc { .. },
                ..
            } = &block.term.kind
            {
                out.push(block.term.span.clone());
            }
        }
    }
    out.sort_by_key(|s| s.begin());
    out
}

fn print_errors<E: biwac_base::BiwacError>(
    errors: &[E],
    interner: &biwac_base::IdentInterner,
    srcs: &biwac_base::SourceHolder,
    metadata: &biwac_base::MetadataHolder,
) {
    let ctx = biwac_base::ErrorContext {
        metadata,
        srcs,
        interner,
    };
    for e in errors {
        e.print_error_message(&ctx);
    }
    biwac_base::print_error_finish_message(errors.len());
}

/// 依存パッケージの生成物を自パッケージの出力ディレクトリに集める。
///
/// 各パッケージは自分の .biwa_build/typescript/<name>.ts に出力するが、
/// codegen が生成する import は `./<package>.ts` という相対パスなので、
/// ルートパッケージの出力ディレクトリに推移的依存も含めて並べる必要がある。
///
/// 再ビルドしたものだけでなく **依存グラフの全パッケージ** が対象である。
/// キャッシュが効いた依存の .ts も並んでいなければならないし、
/// 依存から外れたパッケージの .ts は取り除かなければならない。
fn collect_dep_bins(
    dep_names: &[String],
    self_pkg_name: &str,
    packages_dir: &Path,
    build_dir_path: &Path,
    target: Target,
) -> Result<(), ()> {
    if !target.collects_dependency_binaries() {
        return Ok(());
    }

    let dst_dir = target_dir(build_dir_path, target);
    std::fs::create_dir_all(&dst_dir).map_err(|e| {
        eprintln!("Error: failed to create {:?}: {}", dst_dir, e);
    })?;

    for dep_name in dep_names {
        let file_name = format!("{}.{}", dep_name, target.bin_extension());
        let src = packages_dir
            .join(dep_name)
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join(target.build_subdir())
            .join(&file_name);
        let dst = dst_dir.join(&file_name);

        std::fs::copy(&src, &dst).map_err(|e| {
            eprintln!("Error: failed to copy {:?} to {:?}: {}", src, dst, e);
        })?;
    }

    // グラフから消えたパッケージの生成物を掃除する。
    // 残したままだと、依存を外したのに古いコードが出力に紛れ続ける。
    let ext = target.bin_extension();
    let mut keep: HashSet<String> = dep_names.iter().map(|n| format!("{n}.{ext}")).collect();
    keep.insert(format!("{self_pkg_name}.{ext}"));

    let Ok(entries) = std::fs::read_dir(&dst_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(ext) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !keep.contains(name) {
            let _ = std::fs::remove_file(&path);
        }
    }

    Ok(())
}

fn write_bin(
    build_dir_path: &Path,
    target: Target,
    pkg_name: &PackageName,
    bin: &str,
) -> Result<(), std::io::Error> {
    let path = bin_path(build_dir_path, target, pkg_name.value());
    std::fs::write(path, bin)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    use biwac_base::IdentInterner;
    use biwac_hir::TyKind;
    use biwac_mir::{MirItem, MonoMir, MonoTyDefKind};

    use crate::{BuildOptions, compile};

    thread_local! {
        /// 直近の単相化の結果。
        ///
        /// 単相化の結果はファイルにしないので、テストはここから受け取る。
        pub(super) static LAST_MONO: RefCell<Option<MonoMir>> = const { RefCell::new(None) };
    }

    /// `--emit mir` でビルドし、単相化の結果を返す。
    ///
    /// 単相化は playable パッケージでしか走らないので、
    /// ライブラリに対して呼ぶと `None` になる。
    fn monomorphize(pkg: &str) -> Option<MonoMir> {
        with_build_lock(|built| {
            // 結果はスレッドローカルに置かれるので、同じスレッドで建てて取り出す。
            LAST_MONO.with(|slot| *slot.borrow_mut() = None);
            emit_mir_build(pkg);
            built.insert(pkg.to_string());
            LAST_MONO.with(|slot| slot.borrow_mut().take())
        })
    }

    /// ビルドは常にこの中で行う。
    ///
    /// 同じパッケージを 2 つのテストが同時に建てると
    /// 出力ファイルの書き込みがぶつかるので、直列化する。
    /// `built` には一度建てたパッケージが入り、無駄な建て直しを省く。
    fn with_build_lock<T>(f: impl FnOnce(&mut HashSet<String>) -> T) -> T {
        static BUILT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
        let built = BUILT.get_or_init(|| Mutex::new(HashSet::new()));
        let mut built = built.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut built)
    }

    /// 依存の取得は未実装なので、フィクスチャの分だけここで用意する。
    ///
    /// `<pkg>/.biwa_build/deps/<dep>` に依存パッケージが平らに並んでいることを
    /// ドライバが前提にしている。フィクスチャは同じ `assets/tests` に居るので、
    /// symlink を張れば足りる。コピーにしないのは、テストが
    /// `assets/tests/<dep>/.biwa_build/` に書かれた `.biwamir` を読むためである。
    ///
    /// `.biwa_build` は gitignore されているので、新しいチェックアウトでは
    /// これが無い。テストのたびに作り直す。
    /// TODO: 依存の取得が実装されたらこれを削除する
    fn ensure_fixture_deps(pkg: &str) {
        // (パッケージ, その依存) の表。フィクスチャは数が少ないので直に書く。
        let deps: &[&str] = match pkg {
            "test1" => &["std", "color", "greeter"],
            "greeter" => &["std", "color"],
            "scene_main" => &["std"],
            "main_returning_window" => &["std"],
            "missing_main" => &["std"],
            "uninferable" => &["std"],
            "fn_value"
            | "fn_value_rank1"
            | "fn_value_method"
            | "fn_value_member_conflict"
            | "fn_value_not_callable"
            | "fn_value_trait_item"
            | "fn_value_eq"
            | "fn_value_not_a_method"
            | "fn_lib"
            | "fn_value_capture" => &["std"],
            "fn_user" | "fn_user_scene" => &["std", "fn_lib"],
            "vis_dep_user" | "vis_dep_field_user" | "vis_dep_method_user" => &["vis_dep"],
            _ => &[],
        };
        if deps.is_empty() {
            return;
        }

        let deps_dir = Path::new("../../assets/tests")
            .join(pkg)
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join("deps");
        std::fs::create_dir_all(&deps_dir).expect("failed to create the fixture deps dir");

        for dep in deps {
            let link = deps_dir.join(dep);
            if std::fs::symlink_metadata(&link).is_ok() {
                continue;
            }
            // deps_dir は <pkg>/.biwa_build/deps なので、3 つ上が assets/tests。
            let target = Path::new("../../..").join(dep);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link)
                .unwrap_or_else(|e| panic!("failed to link the fixture dep {dep}: {e}"));
            #[cfg(not(unix))]
            std::os::windows::fs::symlink_dir(&target, &link)
                .unwrap_or_else(|e| panic!("failed to link the fixture dep {dep}: {e}"));
        }
    }

    fn emit_mir_build(pkg: &str) {
        ensure_fixture_deps(pkg);

        compile(
            Path::new("../../assets/tests").join(pkg),
            BuildOptions {
                force_rebuild: true,
                emit_mir: true,
                target: biwac_base::Target::TypeScript,
            },
        )
        .expect("MIR emission failed");
    }

    /// まだ建てていなければ建てる。
    fn build_once(pkg: &str) {
        with_build_lock(|built| {
            if built.insert(pkg.to_string()) {
                emit_mir_build(pkg);
            }
        });
    }

    fn build_dir(pkg: &str) -> PathBuf {
        Path::new("../../assets/tests")
            .join(pkg)
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join(biwac_base::Target::TypeScript.build_subdir())
    }

    /// `--emit mir` でビルドし、書かれた `.biwamir` を返す。
    fn emit_mir_of(pkg: &str) -> String {
        build_once(pkg);
        read_mir(pkg)
    }

    fn read_mir(pkg: &str) -> String {
        std::fs::read_to_string(build_dir(pkg).join(format!("{pkg}.biwamir")))
            .expect("MIR was not written")
    }

    // MIR でしか扱えない構文を含むフィクスチャ。
    //
    // TypeScript の codegen は while 文とブロック文が todo!() のままなので、
    // このパッケージは `--emit mir` でしかビルドできない。
    #[test]
    fn mir_fixture() {
        let mir = emit_mir_of("mir_fixture");

        // while: ループ頭へ戻る後方辺ができる。
        // validator が簡約可能性まで見ているので、
        // ここを通っている時点で後方辺の行き先はループ頭である。
        assert!(mir.contains("switch "), "{mir}");
        assert!(mir.contains("goto "), "{mir}");

        // simplify_cfg が掛かっていること。
        // 文の無い `goto` だけのブロックは畳まれて残らない。
        let empty_goto_blocks = mir
            .lines()
            .zip(mir.lines().skip(1))
            .filter(|(a, b)| {
                a.trim_start().starts_with("bb ") && b.trim_start().starts_with("goto ")
            })
            .count();
        assert_eq!(
            empty_goto_blocks, 0,
            "simplify_cfg should leave no empty goto block:\n{mir}"
        );

        // メンバへの代入は Place の射影になる。
        assert!(mir.contains("_1.count@"), "{mir}");
        // struct literal は集約になる。
        assert!(mir.contains("= agg "), "{mir}");
        // 二項演算は「代入先 = 演算子 被演算子 被演算子」。
        assert!(mir.contains(" = add "), "{mir}");
    }

    // 依存パッケージと scene を含むパッケージ。
    #[test]
    fn test1() {
        let mir = emit_mir_of("test1");

        // scene は普通の関数として落ちる。中断は現れない。
        assert!(!mir.contains("yield"), "{mir}");
        // novel 文は lang item への通常の呼び出しになる。文字列定数を渡している。
        assert!(mir.contains("str:"), "{mir}");

        // 自パッケージのシンボルは `.biwameta` の索引に読み替えられ、
        // ディスク上に SELF (別名の無いパッケージ) は現れない。
        assert!(mir.contains("pkg 0 test1 "), "{mir}");
    }

    /// `.biwamir` を読み戻して書き直すと、元の文字列に一致すること。
    ///
    /// decode の結果は既に `(本当の PackageId, シンボル索引)` の空間にいるので、
    /// 書き直すときの読み替えは恒等になる。
    #[test]
    fn round_trip() {
        // test1 のビルドで std / color / greeter の .biwamir も書かれる。
        build_once("test1");

        for pkg in ["std", "color", "greeter", "test1"] {
            let text = read_mir(pkg);
            let mut interner = IdentInterner::new();
            let decoded = biwac_mir::decode(&text, &mut interner)
                .unwrap_or_else(|e| panic!("failed to decode {pkg}.biwamir: {e}"));

            let again = biwac_mir::encode(
                &decoded.mir,
                &biwac_mir::EncodeCtx {
                    pkg_id: decoded.mir.pkg_id,
                    meta_svh: decoded.meta_svh,
                    // 読み戻した MIR に SELF は無いので、読み替えの表は要らない。
                    symbols: None,
                    interner: &interner,
                },
            );

            assert_eq!(again, text, "{pkg}.biwamir did not round trip");
        }
    }

    /// `.biwamir` から復元した DefId が、`.biwameta` 経由で得られる DefId と一致すること。
    ///
    /// これが崩れると、MIR 上のシンボルと HIR 上のシンボルが別物になり、
    /// 単相化のときに型定義もシグニチャも引けなくなる。
    #[test]
    fn def_ids_match_metadata() {
        build_once("test1");

        let mut interner = IdentInterner::new();

        // greeter の MIR に現れる `std::types::string::String` の TyDefId を取る。
        // greeter::greet は (String) -> String なので、その引数の型がそれである。
        let greeter_text = read_mir("greeter");
        let greeter = biwac_mir::decode(&greeter_text, &mut interner)
            .expect("failed to decode greeter.biwamir")
            .mir;

        // greeter が参照している「greeter 以外のパッケージの型」を集める。
        let mut foreign_tys = std::collections::BTreeSet::new();
        for item in greeter.items.values() {
            let biwac_mir::MirItem::Body(body) = item else {
                continue;
            };
            for local in &body.locals {
                if let biwac_hir::TyKind::Defined(dt) = &local.ty.kind
                    && dt.def_id.pkg() != greeter.pkg_id
                {
                    foreign_tys.insert(dt.def_id.value());
                }
            }
        }
        assert!(
            !foreign_tys.is_empty(),
            "greeter should refer to types from std / color"
        );

        // 同じ型を、依存メタデータ側の経路 (test1 が使っているもの) からも引く。
        // MIR は新しい id を振らないので、両者は同じ値になっていなければならない。
        let meta_path = build_dir("greeter").join("greeter.biwameta");
        let data = std::fs::read(&meta_path).expect("greeter.biwameta is missing");
        let meta = biwac_dependency_metadata::DepMetadata::decode_file(&data)
            .expect("failed to decode greeter.biwameta");

        // greeter.biwamir に書かれた SVH が、いま読んだメタデータのものと一致すること。
        let decoded = biwac_mir::decode(&greeter_text, &mut interner).unwrap();
        assert_eq!(
            decoded.meta_svh, meta.svh,
            "greeter.biwamir was built against a different greeter.biwameta"
        );

        // color::Rgb が greeter の MIR に現れること。
        // test1 は color に依存していないので、
        // 推移閉包のメタデータが揃っていないとこの型は復元できない。
        let color_meta_path = build_dir("color").join("color.biwameta");
        let color_data = std::fs::read(&color_meta_path).expect("color.biwameta is missing");
        let color_meta = biwac_dependency_metadata::DepMetadata::decode_file(&color_data).unwrap();
        let color_pkg = biwac_span::PackageHashId::new(
            &biwac_metadata_loader::try_load_package_metadata(
                Path::new("../../assets/tests/color").to_path_buf(),
            )
            .unwrap()
            .metadata
            .name,
            &biwac_metadata_loader::try_load_package_metadata(
                Path::new("../../assets/tests/color").to_path_buf(),
            )
            .unwrap()
            .metadata
            .version,
        )
        .as_package_id();
        let _ = color_meta;

        assert!(
            foreign_tys
                .iter()
                .any(|v| (*v >> 32) as u32 == color_pkg.value()),
            "greeter's MIR should mention a type owned by color"
        );
    }

    /// 単相化がジェネリクスを消し、到達可能なものだけを残すこと。
    ///
    /// 結果はファイルにしないので、構造をそのまま見る。
    #[test]
    fn monomorphization() {
        let mono = monomorphize("test1").expect("test1 is playable");

        // 同じ関数が複数の実体を持つこと。
        //
        // `scene opening` -> `foo()` は `Pair::new(l, z)` と `Pair::new(x, ...)` を呼び、
        // 前者は [Line, Int]、後者は [Int, Int] になる。
        // 制限つきのジェネリクス (`Wrapper[T: Level]` など) も
        // 満たす型ごとに実体化される。
        //
        // 名前を引く経路をテストに持ち込みたくないので、
        // 「2 つ以上の実体を持つ def_id」として見る。
        // いくつあるかはフィクスチャ次第なので、性質だけを見る。
        let mut by_def: std::collections::HashMap<biwac_span::ValDefId, Vec<&biwac_mir::GenArgs>> =
            std::collections::HashMap::new();
        for inst in &mono.instances {
            by_def
                .entry(inst.key.def_id)
                .or_default()
                .push(&inst.key.args);
        }
        let multi: Vec<_> = by_def.iter().filter(|(_, v)| v.len() > 1).collect();
        assert!(
            !multi.is_empty(),
            "a generic function should have several instances"
        );
        for (def_id, args) in &multi {
            // 同じ引数列の実体が 2 つできていたら、実体化の鍵が壊れている。
            for (i, a) in args.iter().enumerate() {
                for b in &args[i + 1..] {
                    assert_ne!(
                        a,
                        b,
                        "val#{} has two instances with the same generic arguments",
                        def_id.value()
                    );
                }
            }
        }

        // どの実体にもジェネリック型が残っていないこと。
        for inst in &mono.instances {
            assert!(
                inst.key.args.iter().all(|(_, ty)| is_concrete(ty)),
                "instance val#{} still has a generic argument",
                inst.key.def_id.value()
            );
            let MirItem::Body(body) = &inst.item else {
                continue;
            };
            assert!(
                body.genargs.is_empty(),
                "a monomorphized body must not declare generic arguments"
            );
            for (i, local) in body.locals.iter().enumerate() {
                assert!(
                    is_concrete(&local.ty),
                    "local _{i} of val#{} is not concrete: {:?}",
                    inst.key.def_id.value(),
                    local.ty.kind
                );
            }
        }

        // 推移的依存 (color) の型が、メンバ付きで並んでいること。
        // test1 は color に依存していないので、
        // 推移閉包の `.biwamir` / `.biwameta` を辿れていないと出てこない。
        //
        // color には struct が Rgb しか無く、メンバは r / g / b の 3 つである。
        let color_pkg = package_id_of("color");
        let rgb = mono.types.iter().find(|t| t.key.def_id.pkg() == color_pkg);
        assert!(
            rgb.is_some(),
            "a type owned by color should be among the monomorphized types"
        );
        let MonoTyDefKind::Struct { members } = &rgb.unwrap().kind else {
            panic!("color::Rgb should be a struct");
        };
        assert_eq!(members.len(), 3, "color::Rgb has r / g / b");

        // 到達しない関数は入らない。
        // test1 自身の `.biwamir` に載っている item のほうが多いはずである
        // (`Line::len` や `Pair::y_int` などは誰からも呼ばれていない)。
        let own_instances = mono
            .instances
            .iter()
            .filter(|i| i.key.def_id.pkg().is_self())
            .count();
        let own_items = read_mir("test1")
            .lines()
            .filter(|l| l.starts_with("fn ") || l.starts_with("native "))
            .count();
        assert!(
            own_instances < own_items,
            "unreachable functions must be dropped ({own_instances} instances vs {own_items} items)"
        );

        // エントリポイントが自パッケージの scene であること。
        let entry = mono.entry_instance().expect("entry point");
        assert!(entry.key.def_id.pkg().is_self());
        assert!(
            entry.key.args.is_empty(),
            "a scene takes no generic argument"
        );

        // 2 回走らせて同じ並びになること。
        // 実体の索引がそのまま番号になるので、順序が揺れると成果物も揺れる。
        let again = monomorphize("test1").expect("test1 is playable");
        let keys = |m: &MonoMir| {
            format!(
                "{:?}",
                m.instances.iter().map(|i| &i.key).collect::<Vec<_>>()
            )
        };
        assert_eq!(keys(&mono), keys(&again), "instance order must be stable");
        let ty_keys =
            |m: &MonoMir| format!("{:?}", m.types.iter().map(|t| &t.key).collect::<Vec<_>>());
        assert_eq!(ty_keys(&mono), ty_keys(&again), "type order must be stable");
    }

    /// wasm ターゲットで、検証を通る wasm が出ること。
    #[test]
    fn wasm_output() {
        let root = Path::new("../../assets/tests/test1");
        with_build_lock(|_| {
            compile(
                root.to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: false,
                    target: biwac_base::Target::Wasm,
                },
            )
            .expect("wasm build failed");
        });

        let dir = root
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join(biwac_base::Target::Wasm.build_subdir());

        // .wat はデバッグ用に残る。
        let wat = std::fs::read_to_string(dir.join("test1.wat")).expect(".wat was not written");
        assert!(wat.starts_with("(module"), "{wat}");
        // ホスト関数の import 名は std のソースが決めている。
        assert!(
            wat.contains("(import \"biwa:engine\" \"sys_content_push_text\""),
            "{wat}"
        );
        // 単相化されているので、同じ struct の複数の実体が別々の型になる。
        assert!(wat.contains("(rec"), "{wat}");
        assert!(wat.contains("struct.new"), "{wat}");
        // エントリポイント `fn main()` が export される。ランタイムが名前で呼ぶのはこれだけである。
        assert!(wat.contains("(export \"__biwa_entrypoint\""), "{wat}");
        assert!(!wat.contains("__biwa_on_new_game"), "{wat}");
        assert!(!wat.contains("__biwa_app"), "{wat}");
        assert!(!wat.contains("__biwa_std_window_show"), "{wat}");
        // `[[host_export="..."]]` が付いた関数も export される。
        // main から辿れないので、これが出ているのは
        // 単相化の roots に host export が正しく加わっている証拠でもある。
        assert!(wat.contains("(export \"host_export_demo\""), "{wat}");
        // std の `GameWindow` を組み立てる入口。ランタイムはこれで作った値を
        // `SceneStartButton` の `on_click` に渡す。std (依存) の host export なので、
        // test1 の生成物から出ていることがパッケージ越しの export の実用上の確認になる。
        assert!(
            wat.contains("(export \"__biwa_std_game_window_new\""),
            "{wat}"
        );
        // 依存パッケージ (greeter) で `[[host_export="..."]]` が付いた関数も、
        // それを使う側である test1 の生成物から export される。
        // test1 はこれを呼んでいないので、依存由来の host export も
        // roots に入っていることの確認にもなる。
        assert!(
            wat.contains("(export \"greeter_host_export_demo\""),
            "{wat}"
        );

        // .wasm は検証を通ったものである
        // (通っていなければ compile がエラーになっている)。
        let binary = std::fs::read(dir.join("test1.wasm")).expect(".wasm was not written");
        assert_eq!(&binary[..4], b"\0asm", "not a wasm binary");
        assert!(binary.len() > 100, "suspiciously small wasm output");
    }

    /// ライブラリは wasm の成果物を持たないこと。
    ///
    /// 単相化はプログラム全体の操作で、根を持つのは playable だけである。
    #[test]
    fn library_has_no_wasm_output() {
        let root = Path::new("../../assets/tests/std");
        with_build_lock(|_| {
            compile(
                root.to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: false,
                    target: biwac_base::Target::Wasm,
                },
            )
            .expect("wasm build of a library should succeed");
        });

        let dir = root
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join(biwac_base::Target::Wasm.build_subdir());
        assert!(
            dir.join("std.biwamir").is_file(),
            "the MIR is still produced"
        );
        assert!(
            !dir.join("std.wasm").exists(),
            "a library must not produce a wasm binary"
        );
    }

    /// ライブラリでは単相化が走らないこと (根が無い)。
    #[test]
    fn library_is_not_monomorphized() {
        assert!(
            monomorphize("mir_fixture").is_none(),
            "a library package has no entry point, so nothing is monomorphized"
        );
        // `.biwamir` は今までどおり書かれる。
        assert!(!read_mir("mir_fixture").is_empty());
    }

    fn package_id_of(pkg: &str) -> biwac_base::PackageId {
        let metadata = biwac_metadata_loader::try_load_package_metadata(
            Path::new("../../assets/tests").join(pkg),
        )
        .unwrap();
        biwac_span::PackageHashId::new(&metadata.metadata.name, &metadata.metadata.version)
            .as_package_id()
    }

    fn is_concrete(ty: &biwac_hir::Ty) -> bool {
        match &ty.kind {
            TyKind::Int | TyKind::Float | TyKind::Bool | TyKind::Void => true,
            TyKind::Gen(_) | TyKind::LocGen(_) | TyKind::Infer(_) => false,
            TyKind::Defined(dt) => dt.genargs.iter().all(is_concrete),
            TyKind::Fn(f) => f.args.iter().all(is_concrete) && is_concrete(&f.rty),
        }
    }

    /// `[[host_export="..."]]` が `.biwameta` に載り、依存側から読み戻せること。
    ///
    /// ヘッダのフラグが立っている関数だけが、export 名付きで返る。
    #[test]
    fn host_export_is_recorded_in_metadata() {
        build_once("test1");

        let data = std::fs::read(build_dir("greeter").join("greeter.biwameta")).unwrap();
        let meta = biwac_dependency_metadata::DepMetadata::decode_file(&data).unwrap();

        let greeter = package_id_of("greeter");
        let exports = meta
            .host_exports(greeter)
            .expect("host exports should be readable");
        let names: Vec<&str> = exports.iter().map(|(_, name)| *name).collect();
        assert_eq!(names, ["greeter_host_export_demo"]);

        // 返る DefId は依存側から見た greeter のものである。
        let (def_id, _) = exports[0];
        assert_eq!(def_id.pkg(), greeter);

        // 読み戻したボディ (export 名を含む) から計算し直した SVH が、
        // 書いたときの SVH と一致する。
        assert_eq!(meta.compute_svh(), meta.svh);
    }

    /// 以前のエントリポイントの形 (`scene main`) の playable package が拒否されること。
    ///
    /// エントリポイントは `fn main()` だけで、scene は `Window` の `main_scene` として渡す。
    /// `main` という名前の scene は「`main` が関数でない」として拒否する。
    /// フィクスチャの本体は型としては正しく、失敗の理由は契約違反だけである。
    #[test]
    fn rejects_scene_main() {
        ensure_fixture_deps("scene_main");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/scene_main").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(result.is_err(), "`scene main` must be rejected");
    }

    /// `Window` を返す `fn main()` の playable package が拒否されること。
    ///
    /// `Window[S]` は状態の型 `S` を持つのでホストには渡せない。`main` は戻り値を持たず、
    /// 中で `show()` する。フィクスチャの本体は型としては正しく、失敗の理由はシグニチャ検査だけである。
    #[test]
    fn rejects_main_returning_window() {
        ensure_fixture_deps("main_returning_window");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/main_returning_window").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "`fn main()` returning a `Window` must be rejected"
        );
    }

    /// `fn main()` を持たない playable package が拒否されること。
    ///
    /// ランタイムは起動時に `main()` を呼ぶ。フィクスチャは以前の入口の名前 `app` で書いてあり、
    /// 他の部分は正しく、失敗の理由は `main` の欠落だけである。
    #[test]
    fn rejects_playable_without_main() {
        ensure_fixture_deps("missing_main");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/missing_main").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "a playable package without `fn main()` must be rejected"
        );
    }

    /// `mod` 宣言・可視性の構文・`super::` パスを使ったパッケージがコンパイルできること
    /// (issue #8 の段階 1。可視性はまだ検査しない)。
    ///
    /// `super::` は import・型・式のどこにも書け、`super::super::` で 2 つ上を指す。
    #[test]
    fn compiles_mod_tree_with_super_paths() {
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/mod_tree").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(result.is_ok(), "mod_tree must compile");
    }

    /// 可視性が `.biwameta` を通って依存する側に届くこと (issue #8 の段階 2)。
    ///
    /// 書かれた形 (`pub` / `pub(package)` / `pub(super)` / 何も書かない) と、
    /// 依存する側で組み立てた見える範囲を、名前の表 (`DepMetadataModuleView`) と
    /// 型の定義 (`get_ext_ty_impl`) の両方から確かめる。
    /// 可視性を書けない項目は持ち主と同じ (variant は enum)、trait impl の項目は `pub` になる。
    #[test]
    fn visibility_round_trips_through_biwameta() {
        use biwac_dependency_metadata::{DepMetadata, DepMetadataModuleView, PackageModuleView};
        use biwac_hir::{DeclaredVisibility as D, VisibilityScope as S};

        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/mod_tree").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(result.is_ok(), "mod_tree must compile");

        let data = std::fs::read(
            Path::new("../../assets/tests/mod_tree")
                .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
                .join(biwac_base::Target::Wasm.build_subdir())
                .join("mod_tree.biwameta"),
        )
        .unwrap();
        let meta = std::sync::Arc::new(DepMetadata::decode_file(&data).unwrap());
        // 依存する側から見た番号。何でもよい。
        let pkg = biwac_base::PackageId::new(42);
        let module_id = |sym: u32| biwac_base::ModId::new_ext(pkg.value(), sym);

        let mut interner = biwac_base::IdentInterner::default();
        let names = [
            "a", "b", "c", "top", "total", "Pair", "Choice", "Measure", "from_a", "from_b",
            "from_c", "new", "sum", "measure", "Left", "left", "right", "hidden",
        ]
        .map(|n| (n, interner.get_or_insert(n)));
        let id = |n: &str| names.iter().find(|(k, _)| *k == n).unwrap().1;

        let root = DepMetadataModuleView::new_root(std::sync::Arc::clone(&meta), pkg);
        let root_mod = module_id(meta.root_sym_idx);
        let child = |view: &dyn PackageModuleView, n: &str| {
            view.lookup_child(id(n), &interner)
                .unwrap_or_else(|| panic!("`{n}` not found"))
        };
        let check = |r: biwac_dependency_metadata::ExternalChildRef, d: D, s: S| {
            assert_eq!((r.vis.declared, r.vis.scope), (d, s), "{r:?}");
        };

        // ルートモジュールの子。
        let a = child(&root, "a");
        check(a, D::Public, S::Public);
        check(child(&root, "c"), D::Private, S::Module(root_mod));
        check(child(&root, "top"), D::Public, S::Public);
        let pair = child(&root, "Pair");
        check(pair, D::Package, S::Package(pkg));
        let choice = child(&root, "Choice");
        check(choice, D::Public, S::Public);
        check(child(&root, "Measure"), D::Public, S::Public);

        // `pub(super)` はその親 (ここではルート) が範囲になる。
        let a_view = root.get_module_view(a.sym_idx);
        check(
            child(a_view.as_ref(), "from_a"),
            D::Super,
            S::Module(root_mod),
        );
        check(child(a_view.as_ref(), "b"), D::Public, S::Public);
        let c_view = root.get_module_view(child(&root, "c").sym_idx);
        check(
            child(c_view.as_ref(), "from_c"),
            D::Super,
            S::Module(root_mod),
        );

        // 関連 item と variant。
        let assoc = |ty: u32, n: &str| {
            root.lookup_assoc(ty, id(n), &interner)
                .unwrap_or_else(|| panic!("`{n}` not found"))
        };
        check(assoc(pair.sym_idx, "new"), D::Public, S::Public);
        check(assoc(pair.sym_idx, "sum"), D::Package, S::Package(pkg));
        if let Some(measure) = root.lookup_assoc(pair.sym_idx, id("measure"), &interner) {
            check(measure, D::Public, S::Public);
        }
        check(assoc(choice.sym_idx, "Left"), D::Public, S::Public);

        // struct のメンバ。
        let pair_impl = meta
            .get_ext_ty_impl(pair.sym_idx, pkg, &mut interner)
            .unwrap();
        let Some(biwac_hir::TyDefKind::Struct(pair_def)) = &pair_impl.ty_content else {
            panic!("Pair is not a struct")
        };
        assert_eq!(pair_def.vis.declared, D::Package);
        let member = |n: &str| {
            let v = pair_def.member_vis[&id(n)];
            (v.declared, v.scope)
        };
        assert_eq!(member("left"), (D::Public, S::Public));
        assert_eq!(member("right"), (D::Package, S::Package(pkg)));
        assert_eq!(member("hidden"), (D::Private, S::Module(root_mod)));
    }

    /// 依存の無いライブラリのフィクスチャ `dep` を wasm でビルドし、依存パッケージとして返す。
    fn built_dependency(
        dep: &str,
        interner: &mut biwac_base::IdentInterner,
    ) -> biwac_dependency_metadata::ExternalPackage {
        let root = Path::new("../../assets/tests").join(dep);
        let result = with_build_lock(|_| {
            compile(
                root.clone(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(result.is_ok(), "{dep} must compile");

        let metadata = biwac_metadata_loader::try_load_package_metadata(root.clone()).unwrap();
        let data = std::fs::read(
            root.join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
                .join(biwac_base::Target::Wasm.build_subdir())
                .join(format!("{dep}.biwameta")),
        )
        .unwrap();
        biwac_dependency_metadata::ExternalPackage {
            ident: interner.get_or_insert(dep),
            pkg_id: biwac_span::PackageHashId::new(
                &metadata.metadata.name,
                &metadata.metadata.version,
            )
            .as_package_id(),
            meta: std::sync::Arc::new(
                biwac_dependency_metadata::DepMetadata::decode_file(&data).unwrap(),
            ),
            direct: true,
        }
    }

    /// フィクスチャ `pkg` を読み込んで名前解決する。依存は `deps` (ビルド済みのもの) を使う。
    fn resolve_fixture(
        pkg: &str,
        deps: Vec<biwac_dependency_metadata::ExternalPackage>,
        interner: &mut biwac_base::IdentInterner,
    ) -> Result<biwac_name_resolver::ResolveOutput, Vec<biwac_name_resolver::ResolveError>> {
        let mut srcs = biwac_base::SourceHolder::default();
        let root = Path::new("../../assets/tests").join(pkg);
        let metadata = biwac_metadata_loader::try_load_package_metadata(root.clone()).unwrap();
        let mut loaded = biwac_package_loader::Pkg::try_load::<
            biwac_package_loader::BiwacSourceParser,
        >(&metadata, interner, &mut srcs, root)
        .unwrap_or_else(|_| panic!("failed to load {pkg}"));
        let pkg_name = interner.get_or_insert(pkg);
        biwac_name_resolver::NameResolver::new(&metadata, deps, pkg_name, &mut loaded)
            .unwrap()
            .try_resolve(interner)
    }

    /// フィクスチャ `pkg` を名前解決・型推論し、型推論のエラーを `check` に渡す。
    /// 依存は `deps` の名前のフィクスチャ (依存の無いライブラリ) をビルドして使う。
    fn check_ty_error_of(
        pkg: &str,
        deps: &[&str],
        check: impl FnOnce(&biwac_type_inferrer::TyError, &biwac_base::IdentInterner),
    ) {
        let mut interner = biwac_base::IdentInterner::default();
        let deps: Vec<_> = deps
            .iter()
            .map(|d| built_dependency(d, &mut interner))
            .collect();
        let ext = deps
            .iter()
            .map(|d| (d.pkg_id, std::sync::Arc::clone(&d.meta)))
            .collect();
        let resolved = resolve_fixture(pkg, deps, &mut interner)
            .unwrap_or_else(|e| panic!("{pkg} must pass name resolution: {e:?}"));
        let report = match biwac_type_inferrer::TyCtx::new(
            resolved.hir,
            resolved.lang_items,
            ext,
            &mut interner,
        )
        .infer()
        {
            Ok(_) => panic!("{pkg} must be rejected by type inference"),
            Err(report) => report,
        };
        check(&report.error, &interner);
    }

    /// 依存パッケージの項目は `pub` のものしか見えないこと (issue #8 の段階 3)。
    ///
    /// `vis_dep` を実際にビルドして `.biwameta` を作り、それを依存として `vis_dep_user` を名前解決する。
    /// private・`pub(package)`・private なモジュールの中の `pub` が、それぞれ `InvisibleItem` になる。
    #[test]
    fn dependency_items_other_than_pub_are_invisible() {
        let mut interner = biwac_base::IdentInterner::default();
        let dep = built_dependency("vis_dep", &mut interner);
        let errors = match resolve_fixture("vis_dep_user", vec![dep], &mut interner) {
            Ok(_) => panic!("vis_dep_user must be rejected"),
            Err(errors) => errors,
        };
        let mut invisible: Vec<&str> = errors
            .iter()
            .map(|e| match e {
                biwac_name_resolver::ResolveError::InvisibleItem { segment, .. } => {
                    interner.get_str(&segment.ident.id).unwrap()
                }
                e => panic!("unexpected error: {e:?}"),
            })
            .collect();
        invisible.sort();
        assert_eq!(invisible, ["hidden", "in_pkg", "inner"]);
    }

    /// 型推論で、見えないフィールド・メソッドが `InvisibleMember` になること (issue #8 の段階 4)。
    ///
    /// フィクスチャはどれも、子モジュール `a` の `Point` (private なメンバ `y`・関数型のメンバ `run`・
    /// private なメソッド `secret`) を親から使う。型推論は最初のエラーで止まるので、負例は 1 つずつ置いてある。
    #[test]
    fn invisible_members_are_rejected() {
        use biwac_type_inferrer::TyError;

        // 見えるもの (pub なメンバ、親からの pub(super) なメンバ、pub なメソッド) だけなら通る。
        let mut interner = biwac_base::IdentInterner::default();
        let resolved = resolve_fixture("vis_members", Vec::new(), &mut interner)
            .unwrap_or_else(|e| panic!("vis_members must pass name resolution: {e:?}"));
        assert!(
            biwac_type_inferrer::TyCtx::new(
                resolved.hir,
                resolved.lang_items,
                Vec::new(),
                &mut interner
            )
            .infer()
            .is_ok(),
            "vis_members must pass type inference"
        );

        for (pkg, deps, name, method) in [
            ("vis_field_read", &[][..], "y", false),
            ("vis_field_call", &[][..], "run", false),
            ("vis_method", &[][..], "secret", true),
            ("vis_dep_field_user", &["vis_dep"][..], "b", false),
            (
                "vis_dep_method_user",
                &["vis_dep"][..],
                "hidden_method",
                true,
            ),
        ] {
            check_ty_error_of(pkg, deps, |e, interner| match e {
                TyError::InvisibleMember {
                    member, is_method, ..
                } => {
                    assert_eq!(interner.get_str(&member.id), Some(name), "{pkg}");
                    assert_eq!(*is_method, method, "{pkg}");
                }
                e => panic!("{pkg}: unexpected error: {e:?}"),
            });
        }
    }

    /// 見えないメンバのある struct は、struct リテラルで作れないこと (書いたメンバが見えていても)。
    #[test]
    fn struct_literal_with_invisible_field_is_rejected() {
        check_ty_error_of("vis_struct_literal", &[], |e, interner| match e {
            biwac_type_inferrer::TyError::InvisibleFieldInLiteral { field, .. } => {
                // メンバは名前順に見るので、最初に見つかるのは `run` (`y` より前)。
                assert_eq!(interner.get_str(field), Some("run"));
            }
            e => panic!("unexpected error: {e:?}"),
        });
    }

    /// ルートモジュールの `pub(super)` は名前解決のエラーになること (親が無いため)。
    #[test]
    fn rejects_super_visibility_in_root() {
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/super_vis_in_root").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "`pub(super)` in the root module must be rejected"
        );
    }

    /// ルートモジュールで `super::` を使うと名前解決のエラーになること。
    #[test]
    fn rejects_super_beyond_root() {
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/super_beyond_root").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "`super::` in the root module must be rejected"
        );
    }

    /// 型がどこからも決まらない式は、コンパイラの panic ではなく型エラーになること。
    ///
    /// ペイロードを持たないジェネリックなバリアントは型引数が決まらないことがある。
    /// かつては型変数のまま MIR の書き出しまで流れて panic していた。
    #[test]
    fn uninferable_type_is_an_error_not_a_panic() {
        ensure_fixture_deps("uninferable");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/uninferable").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "an expression whose type cannot be inferred must be rejected"
        );
    }

    /// 名前付きの関数を値として渡し、関数型の値を呼べること
    /// (`docs/function-as-the-first-class-type-impl-status.md` のステップ 1)。
    ///
    /// wasm では関数型を型付き関数参照にし、`ref.func` で作って `call_ref` で呼ぶ。
    /// 検証を通らない wasm は compile がエラーにする。
    #[test]
    fn fn_value_wasm_output() {
        ensure_fixture_deps("fn_value");
        let root = Path::new("../../assets/tests/fn_value");
        with_build_lock(|_| {
            compile(
                root.to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: false,
                    target: biwac_base::Target::Wasm,
                },
            )
            .expect("wasm build of fn_value failed");
        });

        let dir = root
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join(biwac_base::Target::Wasm.build_subdir());
        let wat = std::fs::read_to_string(dir.join("fn_value.wat")).expect(".wat was not written");
        assert!(wat.contains("ref.func"), "{wat}");
        assert!(wat.contains("call_ref"), "{wat}");
        // `ref.func` で参照する関数は宣言されていなければならない。
        assert!(wat.contains("(elem declare func"), "{wat}");
        // 関数型のメンバは型付き関数参照のフィールドになる。
        assert!(wat.contains("(field $run (mut (ref null $__fn."), "{wat}");
    }

    /// 関数型がパッケージをまたげること (ステップ 6)。
    ///
    /// 依存 (`fn_lib`) の関数型の引数・戻り値・struct のメンバ・enum のペイロード・
    /// 関数型の型エイリアスを、`.biwameta` / `.biwamir` 越しに使う。
    #[test]
    fn fn_types_across_packages() {
        ensure_fixture_deps("fn_lib");
        ensure_fixture_deps("fn_user");
        let root = Path::new("../../assets/tests/fn_user");
        with_build_lock(|_| {
            compile(
                root.to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: false,
                    target: biwac_base::Target::Wasm,
                },
            )
            .expect("wasm build of fn_user failed");
        });

        let dir = root
            .join(biwac_base::BIWA_BUILD_DIRECTORY_NAME)
            .join(biwac_base::Target::Wasm.build_subdir());
        let wat = std::fs::read_to_string(dir.join("fn_user.wat")).expect(".wat was not written");
        assert!(wat.contains("ref.func"), "{wat}");
        assert!(wat.contains("call_ref"), "{wat}");
    }

    /// 依存パッケージの scene も値にできること (`Scene[S]`。`docs/ui-api-impl-status.md` §19 の R2)。
    #[test]
    fn scene_of_a_dependency_as_value() {
        ensure_fixture_deps("fn_lib");
        ensure_fixture_deps("fn_user_scene");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_user_scene").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_ok(),
            "a scene of a dependency must be usable as a value"
        );
    }

    /// 無名関数が外側の局所変数 (引数・`let`・`self`) を参照するとエラーになること。
    #[test]
    fn anonymous_function_capture_is_an_error() {
        ensure_fixture_deps("fn_value_capture");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_capture").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "an anonymous function capturing a local variable must be rejected"
        );
    }

    /// 関数型の値は量化子を持たない (rank 1)。
    /// ジェネリック引数の関数型 `fn(T) -> T` の値を具体の型で呼ぶのは型エラーであること。
    #[test]
    fn fn_value_is_rank1() {
        ensure_fixture_deps("fn_value_rank1");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_rank1").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "calling `f: fn(T) -> T` with `Int` must be rejected"
        );
    }

    /// 受け手付きのメソッド (`c.get`) は値にできないこと (`Counter::get` ならできる)。
    #[test]
    fn bound_method_as_value_is_an_error() {
        ensure_fixture_deps("fn_value_method");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_method").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "a method bound to its receiver used as a value must be rejected"
        );
    }

    /// `self` を取らない関連関数を `x.make(..)` の形で呼ぶのは型エラーであること。
    #[test]
    fn calling_an_associated_function_as_a_method_is_an_error() {
        ensure_fixture_deps("fn_value_not_a_method");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_not_a_method").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "an associated function without `self` called as a method must be rejected"
        );
    }

    /// struct のメンバ名と関連アイテムの衝突が名前解決のエラーになること。
    ///
    /// 1 つの名前空間で一意にしておかないと、`x.bar(..)` がメンバ (関数型) の値の
    /// 呼び出しかメソッドかが決まらない。
    #[test]
    fn struct_member_named_like_assoc_item_is_an_error() {
        ensure_fixture_deps("fn_value_member_conflict");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_member_conflict").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "a struct member named like a method must be rejected"
        );
    }

    /// 関数型でないメンバを `x.n(..)` で呼ぶのは型エラーであること。
    #[test]
    fn calling_a_non_function_is_an_error() {
        ensure_fixture_deps("fn_value_not_callable");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_not_callable").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(result.is_err(), "calling an `Int` member must be rejected");
    }

    /// trait 越しの項目 (`T::make`) を値として使うのは型エラーであること。呼び出しは通る。
    #[test]
    fn trait_item_as_value_is_an_error() {
        ensure_fixture_deps("fn_value_trait_item");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_trait_item").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "`T::make` used as a value must be rejected"
        );
    }

    /// 関数型の値どうしの `==` は型エラーであること。
    ///
    /// 比較の時点では型変数で、後から関数型に決まる場合もすり抜けない
    /// (演算子の型の検査を推論の最後に回している)。
    #[test]
    fn comparing_function_values_is_an_error() {
        ensure_fixture_deps("fn_value_eq");
        let result = with_build_lock(|_| {
            compile(
                Path::new("../../assets/tests/fn_value_eq").to_path_buf(),
                BuildOptions {
                    force_rebuild: true,
                    emit_mir: true,
                    target: biwac_base::Target::Wasm,
                },
            )
        });
        assert!(
            result.is_err(),
            "comparing function values must be rejected"
        );
    }

    /// `.biwamir` と `.biwameta` の対応が崩れていたら読み込みで止まること。
    #[test]
    fn detects_metadata_mismatch() {
        build_once("test1");

        let text = read_mir("greeter");
        let broken = text
            .lines()
            .map(|l| {
                if l.starts_with("meta-svh") {
                    "meta-svh 0000000000000000".to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let mut interner = IdentInterner::new();
        let decoded = biwac_mir::decode(&broken, &mut interner).expect("still decodable");

        let data = std::fs::read(build_dir("greeter").join("greeter.biwameta")).unwrap();
        let meta = biwac_dependency_metadata::DepMetadata::decode_file(&data).unwrap();

        assert_ne!(
            decoded.meta_svh, meta.svh,
            "the mismatch must be visible to the loader"
        );
    }
}
