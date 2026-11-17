export interface PrivateSyncCategories {
  bookmarks: boolean;
  positions: boolean;
}

export interface PrivateSyncSession {
  id: string;
  invitation?: string | null;
  peerIdentity?: string | null;
  peerName?: string | null;
  verificationCode?: string | null;
  localConfirmed: boolean;
  remoteConfirmed: boolean;
  expiresAt: string;
  phase: "waiting" | "verifying" | "confirmed" | "error";
  error?: string | null;
}

export interface PrivateSyncPeer {
  id: string;
  name: string;
  identity: string;
  enabled: boolean;
  automatic: boolean;
  lastSync?: string | null;
  state: "idle" | "syncing" | "error" | "disabled";
  error?: string | null;
}

export interface PrivateSyncAlternative {
  id: string;
  deviceName: string;
  value: string;
}

export interface PrivateSyncConflict {
  id: string;
  title: string;
  field: string;
  alternatives: PrivateSyncAlternative[];
}

export interface PrivateSyncPosition {
  id: string;
  deviceName: string;
  title: string;
  available: boolean;
}

export interface PrivateSyncStatus {
  identity: string;
  enabled: boolean;
  categories: PrivateSyncCategories;
  session: PrivateSyncSession | null;
  peers: PrivateSyncPeer[];
  conflicts: PrivateSyncConflict[];
  positions: PrivateSyncPosition[];
  pending: number;
  error?: string | null;
}
