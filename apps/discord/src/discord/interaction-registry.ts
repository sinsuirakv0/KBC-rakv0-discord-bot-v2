import type {
  MessageComponentInteraction,
  ModalSubmitInteraction,
} from "discord.js";

const MAX_INTERACTIONS = 64;
const INTERACTION_TTL_MS = 15 * 60 * 1_000;

export type StoredInteraction =
  | MessageComponentInteraction
  | ModalSubmitInteraction;

interface RegistryEntry {
  interaction: StoredInteraction;
  expiresAt: number;
}

export class InteractionRegistry {
  private readonly entries = new Map<string, RegistryEntry>();

  public register(interaction: StoredInteraction): boolean {
    this.prune();
    if (this.entries.size >= MAX_INTERACTIONS) {
      return false;
    }
    this.entries.set(interaction.id, {
      interaction,
      expiresAt: Date.now() + INTERACTION_TTL_MS,
    });
    return true;
  }

  public take(interactionId: string): StoredInteraction | undefined {
    this.prune();
    const entry = this.entries.get(interactionId);
    this.entries.delete(interactionId);
    return entry?.interaction;
  }

  public remove(interactionId: string): void {
    this.entries.delete(interactionId);
  }

  private prune(): void {
    const now = Date.now();
    for (const [interactionId, entry] of this.entries) {
      if (entry.expiresAt <= now) {
        this.entries.delete(interactionId);
      }
    }
  }
}
