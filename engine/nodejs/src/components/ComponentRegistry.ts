import type { TextBox } from "./TextBox";

interface Component {
  element: HTMLElement;
  show(): void;
  hide(): void;
}

export class ComponentRegistry {
  private components = new Map<string, Component>();

  register(id: string, component: Component, domLayer: HTMLElement): void {
    this.components.set(id, component);
    domLayer.appendChild(component.element);
  }

  get(id: string): Component {
    const c = this.components.get(id);
    if (!c) throw new Error(`Component "${id}" not found`);
    return c;
  }

  getTextBox(id: string): TextBox {
    const c = this.get(id);
    if (!("setText" in c)) throw new Error(`Component "${id}" is not a TextBox`);
    return c as TextBox;
  }
}
