//! 宣言の可視性 (`docs/symbol-visibility-impl-status.md`)。
//!
//! 名前解決が AST の可視性 (`biwac_ast::Visibility`) を「見える範囲」に直して持たせる。
//! 依存パッケージの宣言は `.biwameta` から読む。
//! 名前解決はパスを辿るときに、型推論はフィールド・メソッドを引くときに [`Visibility::is_visible_from`] で確かめる。

use biwac_base::{ModId, PackageId};

/// 宣言の可視性。書かれた形と、それが指す見える範囲の組。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    /// 書かれた形。`.biwameta` にはこれを書く。
    pub declared: DeclaredVisibility,
    /// 見える範囲。
    pub scope: VisibilityScope,
}

/// 書かれた可視性。
///
/// 可視性を書けない項目 (enum の variant、trait の項目、trait impl の項目) は
/// 持ち主 (enum・trait) と同じものになる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredVisibility {
    /// 何も書かない。
    Private,
    /// `pub(super)`
    Super,
    /// `pub(package)`
    Package,
    /// `pub`
    Public,
}

/// 見える範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityScope {
    /// どこからでも (`pub`)。
    ///
    /// 祖先のモジュールの可視性による頭打ちは、ここではなくパスを辿る側が見る。
    Public,
    /// そのパッケージの中 (`pub(package)`)。
    Package(PackageId),
    /// そのモジュールとその子孫 (何も書かない・`pub(super)`)。
    ///
    /// 何も書かなければ定義したモジュール、`pub(super)` ならその親である。
    /// 関連 item は impl ブロックのあるモジュールを基準にする。
    Module(ModId),
}

impl Visibility {
    /// 書かれた形と、宣言したモジュール (`home`) とその親 (`parent`) から見える範囲を決める。
    ///
    /// `pub(super)` は親が無ければ (ルートモジュールなら) 決まらないので `None`。
    /// ルートモジュールの `pub(super)` は名前解決がエラーにする。
    pub fn resolve(
        declared: DeclaredVisibility,
        pkg_id: PackageId,
        home: ModId,
        parent: Option<ModId>,
    ) -> Option<Self> {
        let scope = match declared {
            DeclaredVisibility::Private => VisibilityScope::Module(home),
            DeclaredVisibility::Super => VisibilityScope::Module(parent?),
            DeclaredVisibility::Package => VisibilityScope::Package(pkg_id),
            DeclaredVisibility::Public => VisibilityScope::Public,
        };
        Some(Self { declared, scope })
    }
}

impl Visibility {
    /// モジュール `from` から見えるか (`docs/symbol-visibility-impl-status.md` §5.3)。
    ///
    /// 見える範囲の部分木に `from` が入っていればよい。`parent_of` はモジュールの親を返す
    /// (ルートモジュールなら `None`)。祖先による頭打ちはここでは見ない
    /// (パスを辿る側が途中のモジュールごとに確かめる)。
    pub fn is_visible_from(&self, from: ModId, parent_of: impl Fn(ModId) -> Option<ModId>) -> bool {
        match self.scope {
            VisibilityScope::Public => true,
            VisibilityScope::Package(pkg_id) => PackageId::new(from.pkg_id_bits()) == pkg_id,
            VisibilityScope::Module(scope) => {
                let mut cur = Some(from);
                while let Some(m) = cur {
                    if m == scope {
                        return true;
                    }
                    cur = parent_of(m);
                }
                false
            }
        }
    }
}

impl From<&biwac_ast::Visibility> for DeclaredVisibility {
    fn from(value: &biwac_ast::Visibility) -> Self {
        match value {
            biwac_ast::Visibility::Private => Self::Private,
            biwac_ast::Visibility::Super(_) => Self::Super,
            biwac_ast::Visibility::Package(_) => Self::Package,
            biwac_ast::Visibility::Public(_) => Self::Public,
        }
    }
}
