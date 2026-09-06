# std のゲーム API

`library/std` がゲーム開発者に見せる層。
エンジンとの境界 (syscall) は `docs/media-object-model.md` にある。
ここはその上に被せる型の付いた API と、書くときに踏む制限をまとめる。

## 層の分かれ方

```
ゲーム (test/test1)
  ↓  Character / Image / CanvasObject とそのチェーン
std::game::character, std::game::image
  ↓  番号を渡す生の呼び出し
std::game::base_engine        create_object / add_transition / start_transitions ...
  ↓  syscall
エンジン (engine/nodejs)
```

`base_engine` はエンジンとの取り決めをそのまま写した層で、
パラメータや遷移の種類は整数である。
その番号に名前を付けるのが `std::game::image` の
`param_x()` / `kind_ease_out()` などで、
普段はさらにその上のチェーン API を使えばよい。

## canvas オブジェクト

```biwa
let bg = Image::new("background.jpg").show_in_canvas(0, 0, 0, 1280, -1, 255);
```

`show_in_canvas(layer, x, y, w, h, alpha)`。
座標は canvas の中央が原点で、x は右が正、**y は上が正**。
位置も回転も画像の中心を基準にする。
`w` / `h` が負なら「指定しない」で、画像の元のサイズから決まる。
片方だけ正ならアスペクトを保つ。

`CanvasObject` が覚えているのは 2 つだけである。

- **積んだ遷移の目標値** (`x`..`theta`)。つまりアニメーション後の値。
  動いている画像の「今の位置」はエンジンからは取れないので、
  立ち絵の差し替え (`Character::change_visual`) はこれを使って
  アニメーションの行き先に新しい画像を出す。
- **パラメータごとの、積んだ遷移の合計時間** (`*_after`)。
  連続した遷移の開始時刻を計算するために持つ。

遷移そのものはエンジン側に積まれるので、std は列を持たない。

## 2 種類のチェーン

### 1 つのパラメータを続けて動かす — `*_then()`

「それから」というニュアンスで `then` が付く。

```biwa
bg.x_then()
  .sin_forever(24, ms(9000))     // 止めずに揺らし続ける
  .animate_free();               // テキストとは無関係に走らせる

bg.alpha_then()
  .sleep_then(ms(300))           // 300ms 置いてから
  .linear_then(0, ms(1200))      // 1.2 秒かけて透明に
  .animate();                    // テキストと同期する演出として発火
```

|                                                                   |                                              |
| ----------------------------------------------------------------- | -------------------------------------------- |
| `linear_then` / `easein_then` / `easeout_then` / `easeinout_then` | 一度限りの遷移                               |
| `sin_then(振幅, 周期, 続ける時間)`                                | 揺らして、時間が来たら止める                 |
| `sin_forever(振幅, 周期)`                                         | 次に同じパラメータを駆動するまで揺らし続ける |
| `stop_then()`                                                     | その時点で止める                             |
| `sleep_then(時間)`                                                | 次の遷移までの間を空ける                     |
| `animate()` / `animate_free()`                                    | 発火 (同期 / 独立)                           |

### 複数のパラメータを一斉に動かす — `and()`

「かつ」というニュアンスで `and` が付く。
時間は最後の `animate_for()` でまとめて決まる。

```biwa
biwa.appear(1, 700, -20, -1, 700, 0)   // 画面の外に透明で置いて
    .and()
    .easeout_x_and(300)                 // 滑り込ませつつ
    .be_visible_and()                   // 現れる
    .animate_for(ms(900));

biwa.and()
    .move_x_and(-260)                   // 左へ歩きながら
    .sin_y_and(24, ms(500))             // 上下に揺れる
    .animate_for(ms(2000));             // 歩き終わると揺れも止まる
```

周期系を `and()` に混ぜると、`animate_for()` の時間で止まる
(内部で `none` を置いている)。
`sin_forever` に当たるものは `and()` にはない。止め時が決まらないためである。

## 発火は「積んだパラメータ」だけを置き換える

`animate()` / `animate_for()` はエンジンの `start_transitions` を呼ぶ。
置き換わるのは**積んだ遷移が触れた (オブジェクト, パラメータ) だけ**なので、
背景のパンの最中にキャラクターが跳ねてもパンは死なない。

`animate()` は sync 印を付けるので、
その演出が走っている間のクリックはまず演出を完了させる。
`animate_free()` は付けないので、クリックの影響を受けない。

## Character

```biwa
let biwa = Character::new("言葉 琵琶", "琵琶", BiwaProps {}, normal, normal);
biwa.appear(1, 700, -20, -1, 700, 0);
biwa.change_visual(smile);
biwa.disappear(ms(1200));
```

立ち絵は `Option[CanvasObject]` で持つ。
まだ出ていないときは `visual()` が「何もしないオブジェクト」を返すので、
登場前に遷移を書いても落ちずに無視されるだけである。

`change_visual` は、std が覚えている遷移の目標値の位置に新しい画像を出してから
古いものを消す。走っている遷移の途中で呼ぶと、その遷移の行き先に飛ぶ。

## Option

enum が入るまでの暫定。TypeScript では `T | null`、wasm では `anyref` である。

```biwa
let found = images.get(1);
if found.is_some() { ... }
found.unwrap()
```

`some` / `none` / `is_some` / `is_none` / `unwrap`。

wasm の `unwrap` は `anyref` から `T` へ落とすので `ref.cast` が要る。
型は単相化ごとに違うため、native の本体に **`%ret%`** と書くと
コンパイラがその単相化での戻り値の型を埋める
(`%param0%`, `%param1%`, … も同様に引数の型になる。
番号は `local.get` と同じで、self があれば 0 が self)。

**制約**: `anyref` に入るのは WasmGC の参照型だけなので、
wasm では `Option[Int]` や `Option[String]` (externref は別の型階層) は使えない。

## Vec

```biwa
let images = Vec::of(a);
images.push(b);
let found = images.get(1);   // Option[T]。範囲外なら none
images.len()
```

TypeScript では素の `Array`、wasm ではホスト (Worker) が持つ JS の配列で、
`sys_vec_*` として Worker 内で完結する syscall になっている。
要素は `anyref` として渡るので、**wasm では要素の型は参照型 (struct) に限る**。
Option と同じ制約である。

`Vec::new()` は要素の型が戻り値にしか現れないため、
文脈から決められない場所では使えない (下記)。
最初の 1 要素から作る `Vec::of()` ならどこでも書ける。

---

## コンパイラ側の制限

std を書く / std を使うときに踏むもの。回避方法つき。

| 制限                                                                                            | 回避                                                                                 |
| ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| 関数の中で一度も型が決まらないジェネリック引数は決まらない (`let v = Vec::new();` で終わり)     | 使うか、戻り値の型かエイリアスで決める                                               |
| `if cond { 式 }` は if **式**なので `else` が要る                                               | 文にするなら最後に `;` を付ける                                                      |
| `await_transitions` / `sleep` は wasm 専用                                                      | TypeScript では `yield` を任意の呼び出しに置けない (`docs/media-object-model.md`)    |
| TypeScript 生成器に `while` / ブロック文 / ブロックを含む if **式** が無い (`todo!()` で落ちる) | tier 1 の wasm では動く。tier 2 では `if cond { return .. } ..` のように文の形に崩す |

`scene` の `{{ }}` の中はノベルテキストなので、`//` から始まる行、
または `//` 以降がコメントになる。

`library/std` の `character.biwa` には
「ジェネリックな型の関連関数・メソッドを呼び出し側のジェネリック引数で呼べない」
ことを避けるための書き方が残っている
(`CharacterBatchChain { chara = self, .. }` の構造体リテラル、
`self.visual().x_then()` の具体型経由)。
この制限はもう無いので、素直な書き方に戻してよい。

### 型エイリアス越しの関連関数

`type CharacterBiwa = Character[BiwaCharacterProps];` に対して
`CharacterBiwa::new(..)` と書ける。エイリアスの連鎖も辿る。

エイリアスに書いた**型引数も呼び出しに伝わる**。

```biwa
type ImageVec = Vec[Image];

let v = ImageVec::new();   // 引数が無くても T = Image に決まる
```

食い違いは呼び出しの場所でぶつかる。

```biwa
type AliasOther = Character[OtherProps];

AliasOther::new("a", "b", BiwaCharacterProps {}, img, img)
//                        ^^^^^^^^^^^^^^^^^^^^^ OtherProps と衝突する
```

エイリアス自身がまだ型引数を取る場合 (`type PairIntT[T] = Pair[Int, T];`) は、
書かれた型が型引数を埋めきっていないので使えない。
この場合は今までどおり引数から推論する。

仕組みは HIR の `Callee::AssocFn { def_id, self_ty }` である。
`self_ty` は**呼び出し位置に書かれた型**で、エイリアスなら
`alias_expansion` が右辺に置き換える。推論はこれをレシーバのように
第 1 引数として `FnSignature::impl_self_ty` と単一化する。

呼び出し位置に型引数を書く構文 (`Vec[Image]::new()`) はまだ無いので、
エイリアスを経由しない `Vec::new()` の `self_ty` は型引数が空のままである。
その場合は単一化に混ぜず、従来どおり引数と文脈から推論する。

### `if` / `while` の条件式に構造体リテラルは置けない

条件式の直後にはブロックの `{` が来るので、
`if flag {` の `{` を構造体リテラルの開始と読むと必ず誤る。
そこで条件式のあいだは構造体リテラルを式の候補から外している。
通常コードと scene の中の `#` コード行の両方で同じ規則である。

括弧の内側では意味が閉じるので、そこでは書ける。

```biwa
if flag { .. }                          // flag は変数として読まれる
if (Flagged { on = TRUE }).on { .. }    // 括弧で囲めば書ける
if takes(Flagged { on = TRUE }) { .. }  // 引数の内側でも書ける
let f = Flagged { on = TRUE };          // 条件式を抜ければ元どおり
```

条件に構造体リテラルをそのまま書くと、`{` 以降がブロックとして読まれた結果
`Expected \`;\`, but found \`,\`` のような位置ずれしたエラーになる。
専用の診断は用意していない。
