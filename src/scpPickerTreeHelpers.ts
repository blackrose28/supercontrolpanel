/** Pure helpers for SCP remote picker tree — safe against bad / cyclic listings. */

export type RemoteBrowseEntryLike = { path: string; kind: string };

export function isStrictRemoteChildPath(
  parentNormalized: string,
  childNormalized: string,
): boolean {
  if (!childNormalized || parentNormalized === childNormalized) {
    return false;
  }
  if (parentNormalized === "/") {
    return childNormalized.startsWith("/") && childNormalized.length > 1;
  }
  return childNormalized.startsWith(`${parentNormalized}/`);
}

function remotePathName(path: string): string {
  const normalized = path.trim().replace(/\/+$/, "");
  if (!normalized || normalized === "/") {
    return "/";
  }
  const parts = normalized.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? normalized;
}

/**
 * Returns normalized, unique directory paths that are strict children of `parentPath`.
 * Drops self-references, duplicates, and paths that are not under the listed folder.
 */
export function uniqueDirectoryChildPathsForTree(
  parentPath: string,
  entries: RemoteBrowseEntryLike[],
  normalize: (p: string) => string,
): string[] {
  const parentNorm = normalize(parentPath) || "/";
  const seen = new Set<string>();
  const out: string[] = [];
  for (const entry of entries) {
    if (entry.kind !== "directory") {
      continue;
    }
    const childNorm = normalize(entry.path);
    if (!childNorm || !isStrictRemoteChildPath(parentNorm, childNorm)) {
      continue;
    }
    if (seen.has(childNorm)) {
      continue;
    }
    seen.add(childNorm);
    out.push(childNorm);
  }
  out.sort((a, b) => remotePathName(a).localeCompare(remotePathName(b)));
  return out;
}
