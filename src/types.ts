export enum FileTypeEnum {
    SONG = 'song',
    ALBUM = 'album',
    ARTIST = 'artist',
    PLAYLIST = 'playlist',
}

export interface URLMeta {
    storefront: string;
    urlType: string;
    id: string;
}

export interface SongResponse {
    data: AutoSong[];
}

export interface AutoSong {
    id: string;
    type: string;
    href: string;
    attributes: SongAttributes;
    relationsShips: Relationships;
}

interface SongAttributes {
    albumName: string;
    hasTimeSyncedLyrics: boolean;
    genreNames: string[];
    trackNumber: number;
    durationInMillis: number;
    releaseDate: string;
    isVocalAttenuationAllowed: boolean;
    isMasteredForItunes: boolean;
    isrc: string;
    artwork: Artwork;
    audioLocale: string;
    composerName: string;
    url: string;
    playParams: PlayParams;
    discNumber: number;
    isAppleDigitalMaster: boolean;
    hasLyrics: boolean;
    audioTraits: string[];
    name: string;
    previews: Previews[];
    artistName: string;
    extendedAssetUrls: Record<string, string>;
}

interface Artwork {
    width: number;
    height: number;
    url: string;
    textColor1: string;
    textColor2: string;
    textColor3: string;
    textColor4: string;
    bgColor: string;
    hasP3: boolean;
}

interface PlayParams {
    id: string;
    kind: string;
}

interface Previews {
    url: string;
}

interface Relationships {
    albums: Relationship;
    artists: Relationship;
}

interface Relationship {
    href: string;
    data: RelationshipData[];
}

interface RelationshipData {
    id: string;
    type: string;
    href: string;
    attributes?: AlbumAttributes;
}

interface AlbumAttributes {
    copyright: string;
    genreNames: string[];
    releaseDate: string;
    upc: string;
    isMasteredForItunes: boolean;
    artwork: Artwork;
    playParams: PlayParams;
    url: string;
    recordLabel: string;
    trackCount: number;
    isCompilation: boolean;
    isPrerelease: boolean;
    audioTraits: string[];
    isSingle: boolean;
    name: string;
    artistName: string;
    isComplete: boolean;
}
