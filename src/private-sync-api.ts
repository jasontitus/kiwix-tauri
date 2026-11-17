import { api, isPrivacyContextCurrent, type PrivacyContext } from "./api";
import { t } from "./i18n";
import type { TabLocation } from "./types";
import type { PrivateSyncCategories, PrivateSyncStatus } from "./private-sync-types";

type PrivateSyncCommands = {
  private_sync_status: Record<string, never>;
  private_sync_configure: { enabled: boolean; categories: PrivateSyncCategories };
  private_sync_pair_start: Record<string, never>;
  private_sync_pair_join: { invitation: string };
  private_sync_pair_confirm: { sessionId: string; peerIdentity: string };
  private_sync_pair_cancel: { sessionId: string };
  private_sync_peer_update: { peerId: string; name?: string; automatic?: boolean; enabled?: boolean };
  private_sync_unpair: { peerId: string };
  private_sync_now: { peerId: string };
  private_sync_resolve: { conflictId: string; alternativeId: string; alternativeIds: string[] };
  private_sync_rebootstrap: { confirmed: true };
  private_sync_clear_positions: { confirmed: true };
  private_sync_resume: { positionId: string };
};

/** The captured generation belongs to the initiating action, never its callback. */
export function privateSyncRequest<K extends keyof PrivateSyncCommands>(
  command: K,
  args: PrivateSyncCommands[K],
  privacy: PrivacyContext,
): Promise<K extends "private_sync_resume" ? TabLocation : PrivateSyncStatus> {
  if (privacy.privateMode || privacy.readOnly || !isPrivacyContextCurrent(privacy)) {
    return Promise.reject(new Error(t("Private device sync is unavailable in private or read-only mode.")));
  }
  return api<K extends "private_sync_resume" ? TabLocation : PrivateSyncStatus>(command, { ...args, privacyGeneration: privacy.generation });
}
