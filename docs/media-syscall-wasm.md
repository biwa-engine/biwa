# Node.js + Wasm Edition での メディア操作系 syscall

syscall は、`std`でBiwaの型システムのために一段ラップされたうえで、
APIの内部実装やサードパーティライブラリから呼び出される。

## Canvas 上のオブジェクト

canvasとは(JSのcanvasそのもののことを指しているのではない。実際に内部実装はJSのcanvasの可能性もある)、
エンジンが提供する描画可能領域(を表す概念)である。
エンジンは必要なコンポーネントを"UI"として提供しており、
canvasは one of UI である(同様にノベルテキスト領域も one of UI として提供される)。

canvasを操作するsyscall APIにおいて、
x軸、y軸の原点はcanvasの中央とし、
x軸は右が正、y軸は上が正(元々のcanvasと逆)とする。

### 画像,動画

`.png`, `.jpg` などの一般的な画像や、
最終的には、`.gif`のようなアニメーションを含む画像、
`.mov`, `.mp3` などの動画も扱えるようにするためのAPI

#### create

```
fn create_image(
  path: String, // 画像のパス文字列
  x: i32, // x座標
  y: i32, // y座標
  w: i32, // 画像幅
  h: i32, // 画像の高さ
  alpha: i32, // 透明度
  theta: i32, // 回転
) -> i32;
```

`w`, `h`について、負であるならば「指定しない」という意味とする。

- 片方のみ正の場合、正の方を基準としてアスペクトを維持してもう片方を決定する。
- 両方正の場合、アスペクトは破壊されうるが値に従って引き延ばす。
- 両方負の場合、画像の元のサイズ(px)で表示する。

`alpha`について、0〜255の8bit値を使用する。
値が小さいほど透明で、値が大きいほど不透明。
255を超えた値は完全に不透明として丸められる。

`theta`について、画面に平行な向きでの回転を表す。
つまり、z軸が回転の軸である。
将来的に他2軸を軸とした回転の導入が検討されている。
値は、1周を360度とする角度法を取る。360度を超えると360度の剰余で計算される。

戻り値`i32`はエンジンにより割り当てられた image id (u32) を返す。
負の場合はエラーである(画像が存在しないなど)。

#### update

変更後の状態を指定し、そこに至るまでの遷移方法(transition)を指定する。
これにより、あり得る限りの豊かなアニメーションを可能にすることを目指す。

transitionの種類はエンジン側が提供し、syscallではその番号(kind)を指定するものとする。
あるパラメータに対するtransitionの発行は kind, value, after, duration の4値からなる。

transition kind は主に2種類に分けられると考える。

- once
  一度限り起こるもの。
  `value`はそのパラメータの遷移後の値を表す。
  - `linear`: 線形に遷移するだけ。
  - `easein`: 素早く遷移を開始して最後はゆっくりと緩やかに遷移後の値になる。
  - `easeout`: `easein`の逆。
  - `easeinout`: `easein`的に中間地点に到達したかと思えば`easeout`的に遷移後の値になる。
  - `easeoutin`: `easeinout`の逆。
- loop
  繰り返し起こるもの。
  変更先を考える種類の「更新」ではなく、定常的なアニメーションを実現する。
  この場合、`value`パラメータは振幅に該当するような量を表す。
  - `sin`: 正弦波。現在の値を基準に、`value`を振幅とした、`duration`が周期である遷移が起こる。
  - 矩形波、三角波、線形に増大していくだけのループなど...

問題は状態更新が可能なパラメータが、
`x`, `y`, `w`, `h`, `alpha`, `theta`, + 追加が検討される回転2軸 + 将来さらに追加される可能性
ということで、8パラメータ程度ある。
1つのパラメータに対してtransitionは4値取るのならば、同時に発行しようとすると32引数のsyscallが生まれてしまう。
これは問題である。

だからといって何種類かのsyscallにパラメータ毎に分けると、
syscallの発行はWasm VMからホスト関数呼び出しであり多少の時間はかかるので、
syscall発行ごとで分割されたパラメータのtransitionが厳密には一致せず、
望んだアニメーションを得られない可能性が生まれる。

いくつか対応を考えた:

1. パラメータの種類を表す param を導入し、1パラメータへの更新発行を5値とする。
   そのうえで2から4程度のパラメータへの更新を受け付けるsyscall

```
fn update_3params_image(
  id: u32,

  // 1つめのパラメータへの更新
  param1: u32,
  kind1: u32,
  value1: i32,
  after1: u32,
  duration1: u32,

  // 2つめのパラメータへの更新
  param2: u32,
  kind2: u32,
  value2: i32,
  after2: u32,
  duration2: u32,

  // 3つめのパラメータへの更新
  param3: u32,
  kind3: u32,
  value3: i32,
  after3: u32,
  duration3: u32,
) -> i32;
```

この場合、3パラメータだと合計16引数で、あまり改善していないという説もある。
また、同時に発行したいパラメータの数に上限を設けるため、構造的に可能なアニメーションの種類を絞っていることになる。

2. transitionの設定と発火を別のsyscallに分ける。
   発火syscallの発行を基準にすべてのtransitionが開始するためパラメータごとのズレが起こらない。
   オブジェクトの内部状態を正しく把握する必要性が高まるが、
   syscallのローレベルAPIはほとんどの場合stdにラップされるため問題ないという見方もある。

```
fn set_transition_image(
  id: u32,
  param: u32,
  kind: u32,
  value: i32,
  after: u32,
  duration: u32,
) -> i32;

fn start_transition_image(
  id: u32,
) -> i32;
```

パラメータごとでtransitionを設定するように`set_transition_image()`を設計してみた。
引数が現実的な個数に収まる。
一方で、複数のパラメータが絡むアニメーションではその個数分syscallを発行する必要があり、処理として重い可能性がある。

その他の話題:

同じパラメータに関して時間的に複数のtransitionを連続させて成るアニメーションを使いたい場合がある。
そのため、複数のtransitionは内部的に保持されており、
`after`や`duration`から計算してtransitionの実行列をスケジューリングできると良い。
そういう意味では`set_`より`add_transition_image()`のほうが命名としては適切かもしれない。

3. Wasm側の構造体をホスト側が触れるようにする。
   構造体に詰めてしまえば引数の数は無視できる。

```
struct ImageTransitionRequest {
  x: ImageTransitionParamRequest,
  y: ImageTransitionParamRequest,
  ...
}

struct ImageTransitionParamRequest {
  kind: u32,
  value: i32,
  after: u32,
  duration: u32,
}
```

#### delete

canvas上にロードされたオブジェクトは、
基本的にエンジンによって自動で削除されることはない
(エンジンが使用中のオブジェクトか否かを知る方法は根本的にない)

画面からtransitionなしに消したいときに `delete_image()` を発行するのはそうだが、
透明度が完全に透明にupdateされている場合でもリソースリークにならないためには `delete_image()` を発行する必要がある

```
fn delete_image(
  id: u32, // 対象の image id
  after: u32, // [milisecond] 何ミリ秒後に削除されるか
) -> i32;
```

戻り値は成功していれば0だが、失敗するとそれ以外の値(特に負の値)を返す。

`after` の存在により、例えば
徐々に透明になるアニメーションや徐々に画面外に抜けるアニメーションで画像を見えなくしたいときにupdateを発行するが
その `after` + `duration` milisecond 後に消えることになるので、
updateのすぐ後に `delete_image(id, after + duration)` を発行することでリソースリークを防ぎつつアニメーションを達成できる。
