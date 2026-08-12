import { invoke } from "@tauri-apps/api/core";

export type VaultState = "uninitialized" | "locked" | "unlocked";

export interface VaultStatus {
  state: VaultState;
  schema_version: number | null;
  message_count: number | null;
}

export interface NewVaultMessage {
  id: string;
  channel_id: string;
  body: string;
  created_at_ms: number;
}

export interface VaultMessage extends NewVaultMessage {}

export type OutboxWorkerState = "paused" | "idle" | "sending" | "deferred";

export interface OutboxWorkerStatus {
  state: OutboxWorkerState;
  pending: number;
  attempted: number;
  acknowledged: number;
  failed: number;
  last_error: string | null;
}

interface VaultCommandError {
  code: string;
  message: string;
}

const previewMessages: VaultMessage[] = [];
let previewState: VaultState = "unlocked";

export function isNativeVaultRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function getVaultStatus(): Promise<VaultStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultStatus>("vault_status");
  }
  return previewStatus();
}

export async function initializeVault(passphrase: string): Promise<VaultStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultStatus>("vault_initialize", { passphrase });
  }
  previewState = "unlocked";
  return previewStatus();
}

export async function unlockVault(passphrase: string): Promise<VaultStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultStatus>("vault_unlock", { passphrase });
  }
  previewState = "unlocked";
  return previewStatus();
}

export async function lockVault(): Promise<VaultStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultStatus>("vault_lock");
  }
  previewState = "locked";
  return previewStatus();
}

export async function getOutboxStatus(): Promise<OutboxWorkerStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<OutboxWorkerStatus>("outbox_status");
  }
  return previewOutboxStatus();
}

export async function retryOutbox(): Promise<OutboxWorkerStatus> {
  if (isNativeVaultRuntime()) {
    return invoke<OutboxWorkerStatus>("outbox_retry");
  }
  return previewOutboxStatus();
}

async function storePreviewMessage(message: NewVaultMessage): Promise<VaultMessage> {
  if (previewState !== "unlocked") {
    throw new Error("Der lokale Vorschau-Vault ist gesperrt.");
  }
  const stored = { ...message };
  previewMessages.push(stored);
  return stored;
}

export async function queueVaultMessage(message: NewVaultMessage): Promise<VaultMessage> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultMessage>("vault_queue_message", { message });
  }
  return storePreviewMessage(message);
}

export async function listVaultMessages(channelId: string): Promise<VaultMessage[]> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultMessage[]>("vault_list_messages", { channelId });
  }
  return previewMessages.filter((message) => message.channel_id === channelId).map((message) => ({ ...message }));
}

export async function searchVaultMessages(channelId: string, query: string): Promise<VaultMessage[]> {
  if (isNativeVaultRuntime()) {
    return invoke<VaultMessage[]>("vault_search_messages", { channelId, query });
  }
  const normalizedQuery = query.trim().toLocaleLowerCase("de");
  return previewMessages
    .filter(
      (message) =>
        message.channel_id === channelId && message.body.toLocaleLowerCase("de").includes(normalizedQuery),
    )
    .map((message) => ({ ...message }));
}

export function vaultErrorMessage(error: unknown): string {
  if (isVaultCommandError(error)) {
    return error.message;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return "Der lokale Vault konnte nicht verarbeitet werden.";
}

function previewStatus(): VaultStatus {
  return {
    state: previewState,
    schema_version: null,
    message_count: previewState === "unlocked" ? previewMessages.length : null,
  };
}

function previewOutboxStatus(): OutboxWorkerStatus {
  return {
    state: previewState === "unlocked" ? "idle" : "paused",
    pending: 0,
    attempted: 0,
    acknowledged: 0,
    failed: 0,
    last_error: null,
  };
}

function isVaultCommandError(error: unknown): error is VaultCommandError {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    typeof error.code === "string" &&
    "message" in error &&
    typeof error.message === "string"
  );
}