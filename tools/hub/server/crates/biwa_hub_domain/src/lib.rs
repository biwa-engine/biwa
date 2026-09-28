mod error;
mod owner;
mod package;
mod repository;
mod value;
mod version;

pub use error::RepoError;
pub use owner::{Owner, OwnerId, VisibleId, VisibleIdError};
pub use package::{NewPackage, Package, PackageId};
pub use repository::{PackageRepository, VersionRepository};
pub use value::{CommitHash, CommitHashError, RepositoryUrl, RepositoryUrlError};
pub use version::{NewVersion, PackageVersionRecord, VersionDependency};

// パッケージ名・バージョンは biwac_base のものをそのまま domain の語彙として使う。
// コンパイラ/CLI/LSP と hub とで「同じ文字列が同じ意味を持つ」ことを型で保証するため。
pub use biwac_base::{PackageName, PackageNameError, PackageVersion, PackageVersionError};
