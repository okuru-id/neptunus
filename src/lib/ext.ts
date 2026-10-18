// Installed extensions as `ext_list` returns them: the manifest plus a few runtime fields.
export type ExtSetting = {
  key: string; type: string; label: string; description?: string; secret?: boolean;
  default?: unknown; options?: string[]; action?: string;
};
export type ExtInfo = {
  id: string; name: string; displayName?: string; version: string; description?: string; author?: string;
  type: string[]; enabled: boolean; iconData: string | null; values: Record<string, unknown>;
  settings?: ExtSetting[]; qualityOptions?: { id: string; label: string; description?: string }[];
  capabilities?: { homeFeed?: boolean };
  searchBehavior?: { enabled?: boolean; placeholder?: string };
  requiredRuntimeFeatures?: string[];
};
// a repository entry (registry.json)
export type RepoExt = {
  id: string; name?: string; display_name?: string; displayName?: string; version: string; description?: string;
  download_url?: string; downloadUrl?: string; icon_url?: string; iconUrl?: string; category?: string;
  sha256?: string; checksum_sha256?: string;
};

export const extName = (e: ExtInfo) => e.displayName || e.name;
export const has = (e: ExtInfo, t: string) => e.type?.includes(t);
/** Needs host features this app does not provide (signed sessions are SpotiFLAC-server specific). */
export const unsupported = (e: { requiredRuntimeFeatures?: string[] }) =>
  (e.requiredRuntimeFeatures ?? []).filter((f) => /^(signedSession|sessionGrant|patternedFileTransform)@/.test(f));

/** "1.10.2" vs "1.9.0" */
export function newer(a: string, b: string) {
  const pa = a.split(".").map(Number), pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d) return d > 0;
  }
  return false;
}
