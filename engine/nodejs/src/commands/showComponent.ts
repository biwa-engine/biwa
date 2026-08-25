import type { ComponentRegistry } from "../components/ComponentRegistry";
import type { Command } from "../types/Command";

export function showComponent(
  id: string,
  registry: ComponentRegistry,
): Command {
  return async () => {
    registry.get(id).show();
  };
}
