import type { Command } from "../types/Command";

export class CommandQueue {
  private queue: Command[] = [];

  push(...cmds: Command[]): void {
    this.queue.push(...cmds);
  }

  async run(): Promise<void> {
    for (const cmd of this.queue) {
      await cmd();
    }
    this.queue = [];
  }
}

