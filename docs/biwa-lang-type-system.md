# Biwa 言語の型システムについて

関数型言語の流れも汲みつつ、
オブジェクト指向などのユーザーフレンドリーな利点のみ取り入れた、
マルチパラダイムな現代的な型システムで、
豊かな表現力と厳密性の高い型安全性を提供することを目指す。

所有権モデルを含まない Rust の型システムのイメージ。

## 型の種類

### プリミティブ型

言語(コンパイラ)に組み込みの型

- `Int`
  32bit 符号付き整数。
- `Uint`
  32bit 符号なし整数。
- `Float`
  32bit 浮動小数点数。
- `Bool`
  真偽値。`TRUE`, `FALSE`のみ取る。
  出力コードにおけるビット幅などの表現方法はターゲットによるが、0が`FALSE`であるべきである。

なお、Wasmターゲットでは厳密にビット幅が反映されるが、
TSターゲットではTSの型システムの表現力的に`number`として扱われる。
WasmがTier1ターゲットであるため、この点の厳密性は見逃す。

### ユーザ定義型

#### 代数的データ型

##### 構造体型 (struct)

```biwa
struct Foo {
  bar: Bar,
  baz: Baz,
}
```

##### 列挙型 (enum)

内部表現はターゲットによるが、
種別を識別するタグと、各種の値を格納するのに必要なメモリサイズの最大値を取ることで、
必要な最小のメモリサイズに抑えるべきである。

`match` 文で exhaustive match が検査される。

実装は `docs/enum-and-match.md` を参照。

```biwa
enum Foo {
  Bar ( Bar ),
  Qux { quux: Quux, quuux: Quuux },
}
```

#### ジェネリクス型

ユーザ定義型ではジェネリック引数を取った型を定義できる。

現在のパーサでは型名のあとの `[]` で囲った中にジェネリック引数を入れることになっている
(将来他の言語同様`<>`に変更される可能性はある)。

```biwa
struct Foo[T] {
  value: T,
  bar: Bar,
}

enum Baz[T] {
  Myao { value: T },
  Nyan { qux: Qux, quux: Int },
}
```

以下のように、関数に対してもジェネリック引数を使うことができる。

```biwa
fn foo[T](t: T, a: Int) -> Int { ... }
```

ジェネリクス型は、ユーザ定義型に対するものも関数に対するものも、
使用されている箇所それぞれの具体の型にあわせて monomorphization される。

#### 型エイリアス

型に別の名前を付けることができる。
ユーザにとってわかりやすくするための機能であり、
エイリアスした型も型システムの上で元の型と本質的に同じに振る舞う。

```biwa
type Foo[T] = Bar[T, Baz];
```

## 型への実装 (impl)

いずれの種類の型も実装(impl)が可能である。

なお、implは同じpackage内で定義された型に対してのみ可能である。
また、プリミティブ型へのimplはstdでのみ許可される。

ジェネリック引数を取る型は、
ジェネリクスのそれぞれの特殊化が衝突しない限り、
同名の異なる引数の関数を定義可能である。

```biwa
struct Foo[T, U] {
  t: T,
  u: u,
  bar: Bar,
}

impl[T, U] Foo[T, U] {
  fn hoge(t: T, u: U) -> Self {
    Self {
      t = t,
      u = u,
      bar = Bar::new(),
    }
  }

  fn fuga(self) -> U {
    self.u
  }
}

impl[U] Foo[Int, U] {
  fn piyo(self, x: Int) -> Int {
    self.t + x
  }
}

impl[U] Foo[Float, U] {
  // Floatで異なる特殊化がされているため、
  // fn Foo[Int, U]::piyo() とは衝突しない
  // 逆に、Foo[T, U]にpiyo()が定義されると、
  // T = Float や T = Int の際に衝突しうるため、
  // fn Foo[Int, U]::piyo(), fn Foo[Float, U]::piyo() ともにエラーになる
  fn piyo(self, x: Float) -> Float {
    self.t * x
  }
}
```

## 型の振る舞い trait

`trait` により、型の振る舞いを表現できる。

`trait` で定義できるシグニチャは、
`self` を含むメソッド形式か、戻り値に`Self`を含む関連関数形式かのいずれかでなければならない。

`trait` は使用している箇所の具体の型でそれぞれ monomorphization される。

```biwa
trait Gyao {
  fn gyao(self) -> Gyoe;

  fn guee(aaa: Aaa) -> Self;
}

// 型 Nyoee が trait Gyao を満たすという意味で(型と同様に) `:` を使って表す
impl Nyoee: Gyao {
  fn gyao(self) -> Gyoe {
    Gyoe::new(self.nyoe.len())
  }

  fn guee(aaa: Aaa) -> Self {
    Nyoee {
      nyoe = aaa.xxx(),
    }
  }
}

// ジェネリクスを trait Gyao を impl した型に限定
impl[T: Gyao] Bbb[T] {
  fn new(aaa: Aaa, ccc: Ccc) -> Self {
    let t = T::guee(aaa); // Gyao を満たすので [T as Gyao]::guee() が呼べる
    let gyoe = t.gyao(); // 同様に [T as Gyao]::gyao() が呼べる
    Self { ddd = t, gyoe = gyoe }
  }
}
```

`trait` は、`trait`自体か実装対象の型のいずれかがそのpackageで定義されていれば実装できる。
つまり、他のpackageで定義された型についても`trait`により独自の関連関数やメソッドを実装を追加可能である。

その関連関数やメソッドを使用する箇所で実装されている `trait` を `import` している必要がある。
trait solver は、関連関数やメソッドがその型の直接的なimplになかった場合に、
初めて `import` されている `trait` のリストから解決を試みる。

### 現在の制限

`trait` の実装は 2 段に分けており、いまは 1 段目まで入っている。
詳細は `docs/trait.md` を参照。

入っているもの。

- `trait` の宣言と `impl Ty: Trait`
- 具体の型に対する trait 越しの関連関数・メソッドの解決
- 孤児則、impl の重複、関連名の衝突、宣言との一致の検査

まだ入っていないもの。

- **ジェネリクスの trait 制限** (`impl[T: Gyao] Bbb[T]`)。
  2 段目で入れる。呼び先が単相化まで決まらないので、
  wasm は単相化で、TypeScript は witness (辞書) を引数で渡して解く
- 既定実装 (trait の項目に本体を書く)
- 関連型、スーパートレイト、blanket impl (`impl[T] T: Foo`)
- `[T as Gyao]::guee()` の明示的な曖昧さ解消構文。
  これが無いため、**1 つの型にぶら下がる関連名は
  (直接 impl・trait impl・enum のバリアントを通じて) 一意でなければならない**
