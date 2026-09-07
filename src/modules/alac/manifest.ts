import { type MirrorEndpoint, mirrorPolicyManager } from './streaming/index.ts'

export function clearMirrorCache(): void {
  mirrorPolicyManager.clearCache()
}

export async function getMirrorEndpoint(
  forceRefresh = false,
  signal?: AbortSignal,
): Promise<MirrorEndpoint> {
  return mirrorPolicyManager.getEndpoint(forceRefresh, signal)
}
