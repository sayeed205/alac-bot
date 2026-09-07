import { catalogService } from './catalog/index.ts'
import type { AppleTrackMetadata } from './types.ts'

export { catalogService } from './catalog/index.ts'

export async function fetchTrackMeta(
  trackId: string,
  storefront = 'us',
): Promise<AppleTrackMetadata> {
  return catalogService.fetchTrackMeta(trackId, storefront)
}

export async function fetchAlbumTracks(
  collectionId: string,
  storefront = 'us',
): Promise<{ album: AppleTrackMetadata; tracks: AppleTrackMetadata[] }> {
  return catalogService.fetchAlbumTracks(collectionId, storefront)
}

export async function fetchArtistTracks(
  artistId: string,
  storefront = 'us',
): Promise<{
  artistId: string
  artistName: string
  tracks: AppleTrackMetadata[]
}> {
  return catalogService.fetchArtistTracks(artistId, storefront)
}

export async function searchItunesCatalog(
  term: string,
  limit = 5,
  storefront = 'us',
): Promise<AppleTrackMetadata[]> {
  return catalogService.searchCatalog(term, limit, storefront)
}
