fn main() {
    // `BIWA_HUB_URL` は `option_env!` でバイナリに焼き込む (`src/client.rs`)。
    // 素の状態だと cargo は環境変数の変化を検知できず、ソースが変わらない限り
    // 古い値のまま再リンクしないことがあるため、明示的に追跡させる。
    println!("cargo:rerun-if-env-changed=BIWA_HUB_URL");

    // `.env` はこのクレート (`tools/hub/client/`) 直下に置く。
    //
    // path 依存として組み込まれるビルドスクリプトの cwd は常にこのクレート自身の
    // ディレクトリになる (`cli`/`compiler` など、どの workspace からビルドされても同じ)
    // ので、ここが唯一の正しい置き場所になる。
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo");
    let env_path = std::path::Path::new(&manifest_dir).join(".env");
    println!("cargo:rerun-if-changed={}", env_path.display());

    // 既にシェルや CI で `BIWA_HUB_URL` が export 済みならそちらを優先し、
    // `.env` の値では上書きしない (`dotenvy::from_path` の既定の挙動)。
    let _ = dotenvy::from_path(&env_path);

    if let Ok(value) = std::env::var("BIWA_HUB_URL") {
        println!("cargo:rustc-env=BIWA_HUB_URL={value}");
    }
}
