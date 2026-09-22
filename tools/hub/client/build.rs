fn main() {
    // `BIWA_HUB_URL` は `option_env!` でバイナリに焼き込む (`src/client.rs`)。
    // 素の状態だと cargo は環境変数の変化を検知できず、ソースが変わらない限り
    // 古い値のまま再リンクしないことがあるため、明示的に追跡させる。
    println!("cargo:rerun-if-env-changed=BIWA_HUB_URL");
}
