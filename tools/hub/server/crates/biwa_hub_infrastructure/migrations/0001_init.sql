create table packages (
    id uuid primary key,
    name text not null unique,
    repository text not null,
    -- Phase 1 はユーザアカウント/認証を実装しないため常に NULL。
    -- 認証実装時に users テーブルへの外部キーを張る。
    owner_id uuid,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

-- version は (major, minor, patch) の組で表す。biwac_base::PackageVersion に合わせて
-- pre-release/build metadata を持たない単純な 3 つ組。
create table package_versions (
    package_id uuid not null references packages (id),
    major integer not null,
    minor integer not null,
    patch integer not null,
    description text,
    commit_hash text not null,
    -- 公開は取り消せない前提なので updated_at は持たない。
    created_at timestamptz not null default now(),
    primary key (package_id, major, minor, patch)
);

create table package_version_dependencies (
    package_id uuid not null,
    major integer not null,
    minor integer not null,
    patch integer not null,
    depends_on_package_id uuid not null references packages (id),
    dep_major integer not null,
    dep_minor integer not null,
    dep_patch integer not null,
    primary key (package_id, major, minor, patch, depends_on_package_id),
    foreign key (package_id, major, minor, patch) references package_versions (
        package_id, major, minor, patch
    )
);
