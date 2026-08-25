import type { ComponentRegistry } from "../components/ComponentRegistry";
import type { Command } from "../types/Command";

export function hideComponent(
  id: string,
  registry: ComponentRegistry,
): Command {
  return async () => {
    registry.get(id).hide();
  };
}
