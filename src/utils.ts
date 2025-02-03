import type { URLMeta } from './types';

export async function getToken(): Promise<string> {
    // Step 1: Fetch the main page to find the JS file
    const mainPageURL = 'https://beta.music.apple.com';
    const mainPageResponse = await fetch(mainPageURL);
    if (!mainPageResponse.ok) {
        throw new Error(
            `Failed to fetch main page: ${mainPageResponse.statusText}`
        );
    }

    const mainPageBody = await mainPageResponse.text();

    // Find the index-legacy JS URI using regex
    const jsFileRegex = /\/assets\/index-legacy-[^/]+\.js/;
    const indexJsUri = mainPageBody.match(jsFileRegex)?.[0];

    if (!indexJsUri) {
        throw new Error('Index JS file not found');
    }

    // Step 2: Fetch the JS file to extract the token
    const jsFileURL = mainPageURL + indexJsUri;
    const jsFileResponse = await fetch(jsFileURL);
    if (!jsFileResponse.ok) {
        throw new Error(
            `Failed to fetch JS file: ${jsFileResponse.statusText}`
        );
    }

    const jsFileBody = await jsFileResponse.text();

    // Extract the token using regex
    const tokenRegex = /eyJh[^"]+/;
    const token = jsFileBody.match(tokenRegex)?.[0];

    if (!token) {
        throw new Error('Token not found in JS file');
    }

    return token;
}

// Function to extract URL metadata
export function extractUrlMeta(inputURL: string): URLMeta | null {
    // Regex to match album, song, and playlist URLs, including full playlist IDs with hyphens
    const reAlbumOrSongOrPlaylist =
        /https:\/\/music\.apple\.com\/(?<storefront>[a-z]{2})\/(?<type>album|song|playlist)\/.*\/(?<id>[0-9a-zA-Z\-.]+)/;

    const matches = inputURL.match(reAlbumOrSongOrPlaylist);

    if (matches && matches.groups) {
        let { storefront, type: urlType, id } = matches.groups;

        // Handle album URLs with the "i" query parameter (song within album)
        if (urlType === 'album') {
            try {
                const urlObj = new URL(inputURL);

                // If the query contains "i", use the "i" value as the song ID
                const songID = urlObj.searchParams.get('i');
                if (songID) {
                    id = songID;
                    urlType = 'song'; // Treat as "song" since "i" parameter is found
                }
            } catch (err) {
                console.error('Error parsing URL:', err);
                return null;
            }
        }

        // Return the parsed metadata, pluralizing the type
        return {
            storefront,
            urlType: `${urlType}s`, // Pluralize to 'albums', 'songs', or 'playlists'
            id,
        };
    }

    return null;
}
