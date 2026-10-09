/**
 * UI Element のツリーと、その DOM への反映。
 *
 * `CanvasObjects` (`engine/canvas/CanvasObjects.ts`) の UI 版にあたる。
 * biwa 側のパラメータを正とし、DOM は毎回の property 設定・push_child で
 * その場で更新する (canvas と違い、時間で変化する値を持たないので射影は要らない)。
 *
 * 設計は `docs/ui-api.md`、実装方針は `docs/ui-api-impl-status.md` にある。
 */

import { TextBox } from "../../components/TextBox";
import { resolveAssetUrl } from "../api/assets";
import {
  ElementKind,
  HandlerKind,
  handlerTarget,
  isKnownElementKind,
  isKnownHandlerKind,
  isKnownNumericProperty,
  isKnownStringProperty,
  PropertyKind,
  Unit,
} from "../api/ui";

/** UI Element 1 つ。 */
interface UiNode {
  readonly id: number;
  readonly kind: number;
  readonly dom: HTMLElement;
  parent: UiNode | null;
  children: UiNode[];
  /** Link のクリック時の遷移先 page_id。Link 以外では使わない。 */
  onClickLink: string | null;
  /** Page 自身の識別子。Page 以外では使わない。 */
  pageId: string | null;
  /**
   * すべての Element が持てる任意の識別子 (`docs/ui-api.md`)。
   * `UIObjects.idsByName` に登録された自分のキーで、
   * 上書き・削除のときに古いキーを引くのに使う。
   */
  idKey: string | null;
  /**
   * Window の ScenePage (`WindowScenePage` property)。Window 以外では使わない。
   * ScenePage は Window の子でもある (隠れていて、scene が始まるときだけ見える)。
   */
  scenePage: UiNode | null;
  /**
   * ScenePage の scene の出力先 (`ScenePageCanvas` / `ScenePageMessageArea`) の ui_id。
   * ScenePage 以外では使わない。指す Element は ScenePage の子孫でなければならない
   * (Window に結びつけるとき・scene を始めるときに確かめる)。
   */
  sceneCanvasId: number | null;
  sceneMessageAreaId: number | null;
  /**
   * `MessageArea` が持つ Message Window の実体。それ以外では `null`。
   * Content API はこれに出力する。
   */
  textBox: TextBox | null;
  /**
   * `text` property を入れる要素。子を持てる Element (Box) では子と分けて持つ
   * (`textContent` を書くと子が消えるため)。まだ設定されていなければ `null`。
   */
  textEl: HTMLElement | null;
  /**
   * 重なりの高さ。親の値を引き継ぎ、`Layers` の子だけは
   * 「Layers の値 + push された順番 (0 始まり)」になる。
   *
   * 子は後から上に足されるだけで間に割って入ることは無い (`docs/ui-api.md` の `Layers`)
   * ので、単純な足し算で足りる。Layers の子の DOM の `z-index` に使う
   * (canvas の描画先も Canvas Element の DOM の中にあるので、同じ重なりに従う)。
   */
  z: number;
  /**
   * 設定されたハンドラ。ハンドラの種類 (`HandlerKind`) → 預けた関数の番号。
   *
   * 関数そのものは持たない (wasm では Worker にあり、ここには番号しか届かない)。
   * Element が消えるときに、ここにある番号を手放す (`destroy`)。
   */
  handlers: Map<number, number>;
}

/**
 * SceneStartButton が押され、scene を始めてほしいという要求。
 *
 * 値はすべて番号で、関数そのものは関数を預かっている側 (wasm は Worker) にある
 * (`docs/host-function-values.md`)。受け取った側は
 * `onClick(GameWindow::new(canvasId, messageAreaId))` で `Game[S]` を作り、
 * `enterScenePage(windowId)` で ScenePage を見せてから `mainScene(game)` を呼ぶ。
 */
export interface SceneStartRequest {
  /** ボタンが属する Window の ui_id。 */
  windowId: number;
  /** ボタンの `on_click` (`fn(GameWindow) -> Game[S]`) の番号。 */
  onClick: number;
  /** Window の `main_scene` (`Scene[S]`) の番号。 */
  mainScene: number;
  /**
   * Window の ScenePage の出力先 (Canvas / MessageArea) の ui_id。そのまま `__biwa_std_game_window_new` に渡す。
   * ScenePage は今は両方を必ず持つ (`sceneOutputsOf`) が、`GameWindow` 側は将来に備えて
   * 「0 は無し」の取り決めのまま (`Option`) にしてある。
   */
  canvasId: number;
  messageAreaId: number;
}

/**
 * UI の木の組み立て方がエンジンとの取り決めに反している。
 *
 * 他の誤り (知らない kind など) はログに出して続けるが、これは続けても
 * ゲームとして成り立たない誤りなので、ゲームを止める
 * (wasm では `vm/wasm/host.ts` が実行を失敗させる。TypeScript ではゲームコードまで投げる)。
 */
export class UiContractError extends Error {
  constructor(message: string) {
    super(`[biwa] ${message}`);
    this.name = "UiContractError";
  }
}

/** 親が子をいくつ持てるか。`docs/ui-api.md` の push_child の規則そのもの。 */
type ChildCapacity = "many" | "single" | "none";

function childCapacity(kind: number): ChildCapacity {
  switch (kind) {
    case ElementKind.Window:
    case ElementKind.Page:
    case ElementKind.ScenePage:
    case ElementKind.Layers:
    case ElementKind.Horizontal:
    case ElementKind.Vertical:
    case ElementKind.HorizontalGrid:
      return "many";
    case ElementKind.Box:
      return "single";
    default:
      // Link / SceneStartButton や、まだ知らない kind はここに来る。子は持てない。
      return "none";
  }
}

/**
 * UI Element の管理。
 *
 * `ui_id → UiNode` を持ち、syscall のたびに対応する DOM を書き換える。
 * canvas と違って Ticker には登録しない (時間で動くパラメータが無いため)。
 */
export class UIObjects {
  private readonly root: HTMLElement;
  private readonly nodes = new Map<number, UiNode>();
  /**
   * `id` property (文字列) → ui_id の逆引き。
   *
   * `docs/ui-api.md`: Page の `canvas`/`message_area` property (Step3) や、
   * ホストが scene 開始前に ui_id を引く仕組みの土台になる。
   */
  private readonly idsByName = new Map<string, number>();
  /**
   * 生きている `MessageArea`。
   *
   * 文字送り (`update`) とクリックでの送りの完了 (`skipMessageAreas`) は
   * すべての MessageArea に効かせる。毎回 `nodes` を舐めないよう別に持つ。
   */
  private readonly messageAreas = new Set<UiNode>();
  /**
   * SceneStartButton が押されたときに scene を始める者。
   *
   * 関数を預かっている側 (wasm は Worker、TypeScript は `main.ts`) が scene を走らせるので、
   * 走らせる側が `setSceneStarter` で差し込む。差し込まれるまでは何もしない。
   */
  private startScene: (request: SceneStartRequest) => void = () => {};
  private nextId = 1;
  /**
   * 要らなくなったハンドラの番号を、関数を預かっている側に返す。
   *
   * 預かっている場所はターゲットで違う (wasm は Worker、TypeScript は `api/handler.ts`)
   * ので、走らせる側が `setHandlerReleaser` で差し込む。差し込まれるまでは何もしない。
   */
  private releaseHandlers: (handles: number[]) => void = () => {};

  constructor(root: HTMLElement) {
    this.root = root;
  }

  /** ハンドラの番号を手放す先を決める (`releaseHandlers` を参照)。 */
  setHandlerReleaser(release: (handles: number[]) => void): void {
    this.releaseHandlers = release;
  }

  /**
   * 新しい ui_id を採る。
   *
   * TypeScript ターゲット用。wasm ターゲットでは Worker が自分で採番する
   * (`CanvasObjects.allocId` と同じ理由)。
   */
  allocId(): number {
    return this.nextId++;
  }

  /**
   * `id` property (文字列) から ui_id を引く。
   *
   * Step3 (Page の `canvas`/`message_area`) やホスト側から使う想定。
   * 見つからなければ `undefined`。
   */
  resolveId(name: string): number | undefined {
    return this.idsByName.get(name);
  }

  /**
   * ui_id が指す `MessageArea` の Message Window を引く。
   *
   * Content API の出力先の解決に使う。MessageArea でなければ名指しで叱って `null`。
   */
  messageArea(id: number): TextBox | null {
    const node = this.nodes.get(id);
    if (node === undefined) {
      console.error(`[biwa] no such ui element (message area): ${id}`);
      return null;
    }
    if (node.textBox === null) {
      console.error(`[biwa] ui element ${id} is not a MessageArea`);
      return null;
    }
    return node.textBox;
  }

  /** SceneStartButton が押されたときに scene を始める者を決める (`startScene` を参照)。 */
  setSceneStarter(start: (request: SceneStartRequest) => void): void {
    this.startScene = start;
  }

  /**
   * Window の ScenePage を見せ、他の Page を隠す。
   *
   * scene を始める側が、`on_click` で `Game[S]` を作った後・`main_scene` を呼ぶ前に呼ぶ。
   */
  enterScenePage(windowId: number): void {
    const window = this.nodes.get(windowId);
    if (window === undefined || window.kind !== ElementKind.Window) {
      console.error(`[biwa] no such ui element (window): ${windowId}`);
      return;
    }
    if (window.scenePage === null) {
      console.error(`[biwa] ui element ${windowId} (Window) has no ScenePage`);
      return;
    }
    for (const child of window.children) {
      if (child.kind === ElementKind.Page) child.dom.style.display = "none";
    }
    window.scenePage.dom.style.display = "";
  }

  /**
   * ui_id が指す `Canvas` の DOM を引く。Canvas でなければ (消えていても) `null`。
   *
   * Canvas API の描画先 (`CanvasSurface`) はこの中に自分の `<canvas>` を置く。
   */
  canvasElement(id: number): HTMLElement | null {
    const node = this.nodes.get(id);
    if (node === undefined || node.kind !== ElementKind.Canvas) return null;
    return node.dom;
  }

  /**
   * すべての MessageArea の文字送りを進める。
   *
   * Ticker から毎フレーム呼ばれる (`main.ts`)。コールバックを
   * MessageArea ごとに生やさないのは、リークを避けるためと、
   * ポーズ・オート・スキップを 1 箇所の時間操作で効かせるためである。
   */
  update(deltaMs: number): void {
    for (const node of this.messageAreas) {
      node.textBox?.update(deltaMs);
    }
  }

  /**
   * すべての MessageArea の文字送りを完了させる。飛ばすものがあったかを返す。
   *
   * クリック待ち (`waitForClick`) が最初に試す。短絡させず全部に効かせる。
   */
  skipMessageAreas(): boolean {
    let skipped = false;
    for (const node of this.messageAreas) {
      if (node.textBox?.skip()) skipped = true;
    }
    return skipped;
  }

  // --- syscall の実体 -----------------------------------------------------

  /**
   * UI Element を作る。
   *
   * `Window` だけは、それ自身を子として積む先が無いので、作った時点で
   * ルートの DOM 層に直接マウントする。他の kind は `pushChild` されるまで
   * どこにも繋がらない (create と push を分けて、property を設定し終えてから
   * アトミックに出現させるための設計 — `docs/ui-api.md`)。
   */
  create(id: number, kind: number): void {
    if (this.nodes.has(id)) {
      console.error(`[biwa] ui element ${id} already exists`);
      return;
    }
    if (!isKnownElementKind(kind)) {
      console.error(`[biwa] unknown ui element kind: ${kind}`);
      return;
    }

    const textBox = kind === ElementKind.MessageArea ? new TextBox() : null;
    const dom = createDom(kind);
    if (textBox !== null) {
      dom.appendChild(textBox.element);
    }

    const node: UiNode = {
      id,
      kind,
      dom,
      parent: null,
      children: [],
      onClickLink: null,
      pageId: null,
      idKey: null,
      scenePage: null,
      sceneCanvasId: null,
      sceneMessageAreaId: null,
      textBox,
      textEl: null,
      z: 0,
      handlers: new Map(),
    };
    this.nodes.set(id, node);
    if (textBox !== null) {
      this.messageAreas.add(node);
    }

    if (kind === ElementKind.Window) {
      this.root.appendChild(node.dom);
    }
    if (kind === ElementKind.SceneStartButton) {
      dom.addEventListener("click", (event) => {
        event.stopPropagation();
        this.handleSceneStartClick(node);
      });
    }
  }

  /** 数値の property を設定する。 */
  setProperty(
    id: number,
    kind: number,
    valU1: number,
    valU2: number,
    valU3: number,
    valU4: number,
    valI1: number,
    valI2: number,
    valF: number,
  ): void {
    const node = this.nodes.get(id);
    if (node === undefined) {
      console.error(`[biwa] no such ui element: ${id}`);
      return;
    }
    if (!isKnownNumericProperty(kind)) {
      console.error(`[biwa] unknown or non-numeric ui property: ${kind}`);
      return;
    }

    if (kind === PropertyKind.WindowScenePage) {
      this.setScenePage(node, valU1);
      return;
    }
    if (
      kind === PropertyKind.ScenePageCanvas ||
      kind === PropertyKind.ScenePageMessageArea
    ) {
      this.setSceneOutput(node, kind, valU1);
      return;
    }

    applyNumericProperty(
      node,
      kind,
      valU1,
      valU2,
      valU3,
      valU4,
      valI1,
      valI2,
      valF,
    );
  }

  /** 文字列を伴う property を設定する。 */
  setPropertyString(
    id: number,
    kind: number,
    valU: number,
    valI: number,
    valF: number,
    valS: string,
  ): void {
    const node = this.nodes.get(id);
    if (node === undefined) {
      console.error(`[biwa] no such ui element: ${id}`);
      return;
    }
    if (!isKnownStringProperty(kind)) {
      console.error(`[biwa] unknown or non-string ui property: ${kind}`);
      return;
    }

    this.applyStringProperty(node, kind, valU, valI, valF, valS);
  }

  /**
   * ハンドラ (預けた関数の番号) を設定する。
   *
   * 同じ種類のハンドラが既にあれば置き換え、古い番号は手放す。
   * 設定できなかった番号も手放す (誰も呼ばないのに関数を握り続けないため)。
   */
  setHandler(id: number, kind: number, handle: number): void {
    const node = this.nodes.get(id);
    if (node === undefined) {
      console.error(`[biwa] no such ui element: ${id}`);
      this.releaseHandlers([handle]);
      return;
    }
    if (!isKnownHandlerKind(kind)) {
      console.error(`[biwa] unknown ui handler kind: ${kind}`);
      this.releaseHandlers([handle]);
      return;
    }
    if (handlerTarget(kind) !== node.kind) {
      console.error(
        `[biwa] ui handler kind ${kind} cannot be set on ui element ${id} (kind ${node.kind})`,
      );
      this.releaseHandlers([handle]);
      return;
    }

    const previous = node.handlers.get(kind);
    node.handlers.set(kind, handle);
    if (previous !== undefined) {
      this.releaseHandlers([previous]);
    }
  }

  /**
   * Element に設定されたハンドラの番号を引く。無ければ `undefined`。
   *
   * 呼ぶのは関数を預かっている側 (wasm では Worker) で、ここは番号を渡すだけである。
   */
  handlerOf(id: number, kind: number): number | undefined {
    return this.nodes.get(id)?.handlers.get(kind);
  }

  /**
   * 子 Element を親に積む。
   *
   * 親が子を
   * - 複数持てるなら末尾に追加
   * - 1 つのみ持てるなら、無ければ追加・あれば既存を削除して置き換え
   * - 持てないなら、子はツリーから削除される (見た目上は何も起こらない)
   */
  pushChild(parentId: number, childId: number): void {
    const parent = this.nodes.get(parentId);
    const child = this.nodes.get(childId);
    if (parent === undefined) {
      console.error(`[biwa] no such ui element (parent): ${parentId}`);
      return;
    }
    if (child === undefined) {
      console.error(`[biwa] no such ui element (child): ${childId}`);
      return;
    }

    if (parent.kind === ElementKind.Window) {
      this.requireSceneSettings(parent);
    }
    // Canvas / MessageArea は std が `new()` の時点で作るので、同じ値を 2 か所に置くと
    // 同じ Element が 2 回積まれる。黙って移すと先の配置から消えるので止める。
    if (
      child.parent !== null &&
      (child.kind === ElementKind.Canvas || child.kind === ElementKind.MessageArea)
    ) {
      throw new UiContractError(
        `ui element ${child.id} (${child.kind === ElementKind.Canvas ? "Canvas" : "MessageArea"}) ` +
        `is already placed in ui element ${child.parent.id}; the same element cannot be placed twice`,
      );
    }

    switch (childCapacity(parent.kind)) {
      case "none":
        this.destroy(child);
        return;
      case "single":
        for (const existing of [...parent.children]) {
          this.destroy(existing);
        }
        this.attach(parent, child);
        return;
      case "many":
        this.attach(parent, child);
        return;
    }
  }

  // --- 内部 -----------------------------------------------------------------

  /**
   * Window がノベルゲームとして scene を始められることを確かめる。
   *
   * scene を映すページ (`scene_page`) と scene 本体 (`main_scene` のハンドラ) は
   * どちらも欠かせない。Page を積んだ時点で表示が始まりボタンが押せるようになるので、
   * Page を積む前に揃っていなければならない。
   * std は `Window::new` の引数で必ず受け取り、Page より先に設定している。
   */
  private requireSceneSettings(window: UiNode): void {
    const missing: string[] = [];
    if (window.scenePage === null) missing.push("`scene_page`");
    if (!window.handlers.has(HandlerKind.WindowMainScene)) missing.push("`main_scene`");
    if (missing.length > 0) {
      throw new UiContractError(
        `ui element ${window.id} (Window) must have ${missing.join(" and ")} ` +
        "set before its pages are pushed",
      );
    }
  }

  private attach(parent: UiNode, child: UiNode): void {
    if (child.parent !== null) {
      this.detachFromParent(child);
    }
    parent.children.push(child);
    child.parent = parent;
    parent.dom.appendChild(child.dom);
    // 子の部分木はすでに組み上がっている (create → property → push_child の順) ので、
    // 繋がった時点で部分木ごと重なりの高さを決め直す。
    this.assignZ(child, zOfChild(parent, parent.children.length - 1));

    if (child.kind === ElementKind.Page) {
      this.showFirstPageIfNoneVisible(parent, child);
    }
  }

  private detachFromParent(node: UiNode): void {
    const parent = node.parent;
    if (parent === null) return;
    parent.children = parent.children.filter((c) => c !== node);
    node.dom.remove();
    node.parent = null;
    if (parent.scenePage === node) {
      parent.scenePage = null;
    }
    if (parent.kind === ElementKind.Layers) {
      node.dom.style.gridArea = "";
      node.dom.style.alignSelf = "";
      node.dom.style.zIndex = "";
    }
  }

  /**
   * `node` を高さ `z` にし、部分木に伝える。
   *
   * Layers の子は、Layers の 1 つのマスに重ねて置き (`grid-area: 1 / 1`)、
   * `z-index` で前後を決める。後から push されたものほど上になる。
   *
   * グリッドの子は既定で縦にも引き伸ばされる (`align-self: stretch`) が、
   * 普通の親の中では高さ指定の無い Element は中身の高さになる。
   * 「Layers の親の中にいるのと同じ配置」にするため、縦は `start` にする
   * (横は普通の親の中と同じく幅いっぱいに伸びる)。
   */
  private assignZ(node: UiNode, z: number): void {
    node.z = z;
    if (node.parent?.kind === ElementKind.Layers) {
      node.dom.style.gridArea = "1 / 1";
      node.dom.style.alignSelf = "start";
      node.dom.style.zIndex = String(z);
    }
    node.children.forEach((child, index) => {
      this.assignZ(child, zOfChild(node, index));
    });
  }

  /** ツリーから消す。子も道連れにする。 */
  private destroy(node: UiNode): void {
    this.detachFromParent(node);
    for (const child of [...node.children]) {
      this.destroy(child);
    }
    if (node.idKey !== null && this.idsByName.get(node.idKey) === node.id) {
      this.idsByName.delete(node.idKey);
    }
    this.messageAreas.delete(node);
    this.nodes.delete(node.id);
    if (node.handlers.size > 0) {
      this.releaseHandlers([...node.handlers.values()]);
      node.handlers.clear();
    }
  }

  private applyStringProperty(
    node: UiNode,
    kind: number,
    _valU: number,
    _valI: number,
    _valF: number,
    valS: string,
  ): void {
    switch (kind) {
      case PropertyKind.PageId:
        node.pageId = valS;
        return;
      case PropertyKind.OnClickLink:
        node.onClickLink = valS;
        this.ensureLinkClickHandler(node);
        return;
      case PropertyKind.Text:
        this.setText(node, valS);
        return;
      case PropertyKind.TextFont:
        node.dom.style.fontFamily = valS;
        return;
      case PropertyKind.BackgroundImage:
        node.dom.style.backgroundColor = "";
        node.dom.style.backgroundImage = `url(${resolveAssetUrl(valS)})`;
        node.dom.style.backgroundSize = "cover";
        node.dom.style.backgroundPosition = "center";
        return;
      case PropertyKind.Image:
        if (node.kind !== ElementKind.Image) {
          console.error(
            `[biwa] "image" property is only meaningful on Image (ui element ${node.id})`,
          );
          return;
        }
        node.dom.style.backgroundImage = `url(${resolveAssetUrl(valS)})`;
        return;
      case PropertyKind.Id:
        this.setId(node, valS);
        return;
      default:
        console.error(`[biwa] unhandled string ui property: ${kind}`);
    }
  }

  /**
   * `text` property を設定する。
   *
   * 子を持たない Element (Link) はそのまま中身を置き換える。子を持てる Box は、
   * 子より前に置いた専用の要素に入れて子を残す。装飾 (`text_font` / `text_size` /
   * `text_weight` / `text_color`) は Element 自身に設定し、テキストはそれを引き継ぐ。
   */
  private setText(node: UiNode, text: string): void {
    if (node.kind !== ElementKind.Box) {
      node.dom.textContent = text;
      return;
    }
    if (node.textEl === null) {
      node.textEl = document.createElement("span");
      node.dom.prepend(node.textEl);
    }
    node.textEl.textContent = text;
  }

  /**
   * Element の `id` property を設定する。すべての Element が持てる
   * (`docs/ui-api.md`)。同じ名前が既に別の Element に使われていれば、
   * 取り違えに気づけるようログだけ出して上書きする。
   */
  private setId(node: UiNode, name: string): void {
    if (node.idKey !== null && node.idKey !== name) {
      // 付け替え。古いキーが今も自分を指しているときだけ消す。
      if (this.idsByName.get(node.idKey) === node.id) {
        this.idsByName.delete(node.idKey);
      }
    }

    const existing = this.idsByName.get(name);
    if (existing !== undefined && existing !== node.id) {
      console.error(
        `[biwa] ui element id "${name}" is already used by element ${existing}; overwriting with ${node.id}`,
      );
    }

    node.idKey = name;
    this.idsByName.set(name, node.id);
    // DOM 上でも見えるようにしておく (devtools・自動テストからの確認用)。
    node.dom.dataset["biwaId"] = name;
  }

  /**
   * Link の click ハンドラを 1 度だけ張る。
   *
   * クリックは host まで伝播させない。伝播させると Message Window の
   * クリック待ち (`waitForClick`) も同時に反応してしまう。
   */
  private ensureLinkClickHandler(node: UiNode): void {
    if (node.dom.dataset["biwaLinkBound"] === "1") return;
    node.dom.dataset["biwaLinkBound"] = "1";
    node.dom.addEventListener("click", (event) => {
      event.stopPropagation();
      this.handleLinkClick(node);
    });
  }

  private handleLinkClick(link: UiNode): void {
    const target = link.onClickLink;
    if (target === null) return;

    const win = ownerWindow(link);
    if (win === null) {
      console.error("[biwa] a Link outside of any Window was clicked");
      return;
    }
    this.showPage(win, target);
  }

  /** Window 直下にまだ見えている Page が無ければ、追加された Page を見せる。 */
  private showFirstPageIfNoneVisible(window: UiNode, page: UiNode): void {
    const alreadyVisible = window.children.some(
      (c) => c.kind === ElementKind.Page && c !== page && isVisible(c),
    );
    if (!alreadyVisible) {
      page.dom.style.display = "";
    }
  }

  /**
   * Window 直下の Page を、page_id が一致するものだけ見せる。
   *
   * ScenePage は Link では遷移できない (page_id を持たない) ので、見えていれば隠すだけである。
   */
  private showPage(window: UiNode, pageId: string): void {
    let found = false;
    for (const child of window.children) {
      if (child.kind === ElementKind.ScenePage) {
        child.dom.style.display = "none";
        continue;
      }
      if (child.kind !== ElementKind.Page) continue;
      const match = child.pageId === pageId;
      child.dom.style.display = match ? "" : "none";
      found ||= match;
    }
    if (!found) {
      console.error(`[biwa] no such page in this window: "${pageId}"`);
    }
  }

  /**
   * Window に ScenePage を結びつける (`WindowScenePage` property)。
   *
   * ScenePage は Window の隠れた子になる。既に別の ScenePage があれば消して置き換える。
   */
  private setScenePage(window: UiNode, scenePageId: number): void {
    if (!expectKind(window, ElementKind.Window, "scene_page")) return;
    const scenePage = this.nodes.get(scenePageId);
    if (scenePage === undefined || scenePage.kind !== ElementKind.ScenePage) {
      console.error(
        `[biwa] "scene_page" of ui element ${window.id} must be a ScenePage, but got ui element ${scenePageId}`,
      );
      return;
    }
    // ScenePage の部分木はこの時点で組み上がっている (std は子から組み上げる) ので、
    // 出力先が揃って子孫に置かれていることをここで確かめる。
    this.sceneOutputsOf(scenePage);
    if (window.scenePage === scenePage) return;
    if (window.scenePage !== null) {
      this.destroy(window.scenePage);
    }
    this.attach(window, scenePage);
    window.scenePage = scenePage;
  }

  /** ScenePage の出力先を設定する (`ScenePageCanvas` / `ScenePageMessageArea`)。 */
  private setSceneOutput(scenePage: UiNode, kind: number, targetId: number): void {
    const isCanvas = kind === PropertyKind.ScenePageCanvas;
    const property = isCanvas ? "canvas" : "message_area";
    if (!expectKind(scenePage, ElementKind.ScenePage, property)) return;
    const expected = isCanvas ? ElementKind.Canvas : ElementKind.MessageArea;
    const target = this.nodes.get(targetId);
    if (target === undefined || target.kind !== expected) {
      throw new UiContractError(
        `"${property}" of ui element ${scenePage.id} (ScenePage) must be a ` +
        `${isCanvas ? "Canvas" : "MessageArea"}, but got ui element ${targetId}`,
      );
    }
    if (isCanvas) {
      scenePage.sceneCanvasId = targetId;
    } else {
      scenePage.sceneMessageAreaId = targetId;
    }
  }

  /**
   * ScenePage の出力先 (Canvas と MessageArea の ui_id) を返す。
   *
   * どちらも設定されていて、ScenePage の子孫に置かれていなければならない。
   * std は ID を `ScenePage::new` で必ず受け取るが、実体の配置は型で担保できないので、ここで確かめる。
   */
  private sceneOutputsOf(scenePage: UiNode): { canvasId: number; messageAreaId: number } {
    const resolve = (id: number | null, property: string): number => {
      if (id === null) {
        throw new UiContractError(
          `ui element ${scenePage.id} (ScenePage) must have its "${property}" set`,
        );
      }
      const target = this.nodes.get(id);
      if (target === undefined || !isDescendantOf(target, scenePage)) {
        throw new UiContractError(
          `the "${property}" of ui element ${scenePage.id} (ScenePage) is ui element ${id}, ` +
          "which must be placed inside the ScenePage",
        );
      }
      return id;
    };
    return {
      canvasId: resolve(scenePage.sceneCanvasId, "canvas"),
      messageAreaId: resolve(scenePage.sceneMessageAreaId, "message_area"),
    };
  }

  /**
   * SceneStartButton が押された。Window の ScenePage の出力先と預けた関数の番号を揃えて、
   * scene を始める者 (`setSceneStarter`) に渡す。
   *
   * クリックは host まで伝播させない (Link と同じ理由)。
   */
  private handleSceneStartClick(button: UiNode): void {
    const onClick = button.handlers.get(HandlerKind.SceneStartButtonOnClick);
    if (onClick === undefined) {
      console.error(`[biwa] SceneStartButton (ui element ${button.id}) has no on_click`);
      return;
    }
    const win = ownerWindow(button);
    if (win === null) {
      console.error("[biwa] a SceneStartButton outside of any Window was clicked");
      return;
    }
    const mainScene = win.handlers.get(HandlerKind.WindowMainScene);
    const scenePage = win.scenePage;
    if (mainScene === undefined || scenePage === null) {
      // Page を積む時点で検査している (`requireSceneSettings`) ので、ここには来ない想定。
      console.error(`[biwa] ui element ${win.id} (Window) cannot start a scene`);
      return;
    }
    // Window に結びつけたときに確かめてあるが、その後に動かされていないとも限らないので見直す。
    // ここはイベントの中なので、誤りはゲームを止めずにログに出すに留める。
    let outputs: { canvasId: number; messageAreaId: number };
    try {
      outputs = this.sceneOutputsOf(scenePage);
    } catch (e) {
      console.error(e);
      return;
    }
    this.startScene({ windowId: win.id, onClick, mainScene, ...outputs });
  }
}

/** `kind` に対応する DOM を作る。中身は空で、property・child は後から付く。 */
function createDom(kind: number): HTMLElement {
  switch (kind) {
    case ElementKind.Window:
      return styled(document.createElement("div"), {
        position: "absolute",
        inset: "0",
        // Window 自身はクリックを奪わない。奪うのは Link と SceneStartButton だけ。
        pointerEvents: "none",
      });
    case ElementKind.Page:
      return styled(document.createElement("div"), {
        position: "absolute",
        inset: "0",
        // 最初は隠れている。表示は `showFirstPageIfNoneVisible` / `showPage` が決める。
        display: "none",
      });
    case ElementKind.ScenePage:
      // Page と同じ。最初は隠れていて、scene が始まるときに `enterScenePage` が見せる。
      return styled(document.createElement("div"), {
        position: "absolute",
        inset: "0",
        display: "none",
      });
    case ElementKind.Box:
      return styled(document.createElement("div"), {
        position: "relative",
      });
    case ElementKind.SceneStartButton:
    case ElementKind.Link:
      return styled(document.createElement("div"), {
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        textAlign: "center",
        cursor: "pointer",
        pointerEvents: "auto",
      });
    case ElementKind.Horizontal:
      return styled(document.createElement("div"), {
        display: "flex",
        flexDirection: "row",
      });
    case ElementKind.Vertical:
      return styled(document.createElement("div"), {
        display: "flex",
        flexDirection: "column",
      });
    case ElementKind.HorizontalGrid:
      return styled(document.createElement("div"), {
        display: "grid",
        gridTemplateColumns: "repeat(1, 1fr)",
      });
    case ElementKind.Image:
      return styled(document.createElement("div"), {
        backgroundSize: "cover",
        backgroundPosition: "center",
        backgroundRepeat: "no-repeat",
      });
    case ElementKind.Layers:
      // 1 マスのグリッドにして、子をすべてそのマスに重ねる (`assignZ`)。
      // 自分は親の内側を埋めるので、子はそれぞれ Layers の親の中にいるのと
      // 同じように配置され、x / y 方向には互いに干渉しない
      // (子の % は親と同じ大きさのマスに対して解決される)。
      // 前後は子の z-index が決める。
      return styled(document.createElement("div"), {
        position: "relative",
        display: "grid",
        gridTemplateColumns: "100%",
        gridTemplateRows: "100%",
        width: "100%",
        height: "100%",
      });
    case ElementKind.Canvas:
      // Canvas API の出力先。描画先 (`CanvasSurface`) が自分の `<canvas>` を
      // この中に置き、この要素の大きさに合わせる。
      return styled(document.createElement("div"), {
        position: "relative",
      });
    case ElementKind.MessageArea:
      // 中に Message Window (`TextBox`) を入れ、TextBox が padding の内側を埋める。
      // 大きさ・背景・余白は property が決める。padding を足しても
      // width / height で決めた枠の大きさが変わらないよう border-box にする。
      return styled(document.createElement("div"), {
        position: "relative",
        boxSizing: "border-box",
      });
    default:
      // `isKnownElementKind` を先に通しているので、ここには来ない想定。
      throw new Error(`[biwa] unknown ui element kind: ${kind}`);
  }
}

function styled(
  el: HTMLElement,
  style: Partial<CSSStyleDeclaration>,
): HTMLElement {
  Object.assign(el.style, style);
  return el;
}

function applyNumericProperty(
  node: UiNode,
  kind: number,
  valU1: number,
  valU2: number,
  valU3: number,
  valU4: number,
  valI1: number,
  valI2: number,
  valF: number,
): void {
  switch (kind) {
    case PropertyKind.Width:
      node.dom.style.width = sizeToCss(valU1, valF);
      return;
    case PropertyKind.Height:
      node.dom.style.height = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginLeft:
      node.dom.style.marginLeft = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginRight:
      node.dom.style.marginRight = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginTop:
      node.dom.style.marginTop = sizeToCss(valU1, valF);
      return;
    case PropertyKind.MarginBottom:
      node.dom.style.marginBottom = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingLeft:
      node.dom.style.paddingLeft = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingRight:
      node.dom.style.paddingRight = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingTop:
      node.dom.style.paddingTop = sizeToCss(valU1, valF);
      return;
    case PropertyKind.PaddingBottom:
      node.dom.style.paddingBottom = sizeToCss(valU1, valF);
      return;
    case PropertyKind.BackgroundColor:
      node.dom.style.backgroundImage = "";
      node.dom.style.backgroundColor = `rgba(${valU1}, ${valU2}, ${valU3}, ${valU4 / 255})`;
      return;
    case PropertyKind.Column:
      if (node.kind !== ElementKind.HorizontalGrid) {
        console.error(
          `[biwa] "column" property is only meaningful on HorizontalGrid (ui element ${node.id})`,
        );
        return;
      }
      node.dom.style.gridTemplateColumns = `repeat(${Math.max(valU1, 1)}, 1fr)`;
      return;
    case PropertyKind.TextSize:
      node.dom.style.fontSize = sizeToCss(valU1, valF);
      return;
    case PropertyKind.TextWeight:
      node.dom.style.fontWeight = String(valU1);
      return;
    case PropertyKind.TextColor:
      node.dom.style.color = `rgba(${valU1}, ${valU2}, ${valU3}, ${valU4 / 255})`;
      return;
    default:
      // `isKnownNumericProperty` を先に通しているので、ここには来ない想定。
      console.error(`[biwa] unhandled numeric ui property: ${kind}`);
  }

  // unused: val_i1 / val_i2 は現行の数値 property (すべて非負) では使わないが、
  // 符号付きの位置指定などを足すときのために syscall のシグネチャには残してある。
  void valI1;
  void valI2;
}

function sizeToCss(unit: number, value: number): string {
  switch (unit) {
    case Unit.Vw:
      return `${value}vw`;
    case Unit.Vh:
      return `${value}vh`;
    case Unit.Percent:
      return `${value}%`;
    default:
      console.error(`[biwa] unknown ui size unit: ${unit}`);
      return "0";
  }
}

/** `parent` の `index` 番目の子の重なりの高さ。Layers の子だけが上に積み上がる。 */
function zOfChild(parent: UiNode, index: number): number {
  return parent.kind === ElementKind.Layers ? parent.z + index : parent.z;
}

/** property を付けてよい Element か。違えば名指しで叱る。 */
function expectKind(node: UiNode, kind: number, property: string): boolean {
  if (node.kind === kind) return true;
  console.error(
    `[biwa] "${property}" property is not meaningful on this element (ui element ${node.id})`,
  );
  return false;
}

function isVisible(node: UiNode): boolean {
  return node.dom.style.display !== "none";
}

/** `node` が `ancestor` の子孫か (自分自身は含まない)。 */
function isDescendantOf(node: UiNode, ancestor: UiNode): boolean {
  let cur = node.parent;
  while (cur !== null) {
    if (cur === ancestor) return true;
    cur = cur.parent;
  }
  return false;
}

/** 祖先を辿って、自分を含む最も近い Window を探す。 */
function ownerWindow(node: UiNode): UiNode | null {
  let cur: UiNode | null = node;
  while (cur !== null) {
    if (cur.kind === ElementKind.Window) return cur;
    cur = cur.parent;
  }
  return null;
}
