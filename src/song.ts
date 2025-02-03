import { InputMedia } from '@mtcute/bun';
import type { filters, MessageContext } from '@mtcute/dispatcher';
import { unlink } from 'node:fs/promises';

import { dlopen, FFIType, ptr, suffix } from 'bun:ffi';
import * as env from './env';
import { File } from './models/file.ts';
import { tg } from './tg';
import {
    FileTypeEnum,
    type AutoSong,
    type SongResponse,
    type URLMeta,
} from './types';
import { extractUrlMeta, getToken } from './utils';

export async function downloadSong(
    msg: filters.Modify<MessageContext, { command: string[] }>
): Promise<void> {
    const args = msg.text.split(' ').slice(1);

    if (args.length === 0) {
        await msg.answerText('Please provide a song URL.');
        return;
    }
    if (args.length > 1) {
        await msg.answerText('Please provide only one song URL.');
        return;
    }

    const url = args[0];
    try {
        const urlMeta = extractUrlMeta(url);
        if (!urlMeta) {
            throw new Error('Invalid URL');
        }
        const token = await getToken();
        const songMeta = await getSongMeta(urlMeta, token);
        if (!songMeta) {
            throw new Error('Failed to get song meta');
        }

        const { cstring } = FFIType;

        const ffi = dlopen(`wrapper.${suffix}`, {
            DownloadSong: {
                args: [cstring],
                returns: cstring,
            },
        });
        // eslint-disable-next-line node/prefer-global/buffer
        const filePath = `${ffi.symbols.DownloadSong(
            ptr(Buffer.from(url.trim() + '\0', 'utf8'))
        )}`;

        if (filePath.startsWith('error')) {
            throw new Error(filePath.split('::')[1]);
        }
        const song = Bun.file(filePath);
        if (await song.exists()) {
            const uploadMessage = await msg.answerText('Uploading song...');
            let lastUpdate = 0;

            const songMsg = await tg.sendMedia(
                Number(env.DUMP_ID),
                InputMedia.audio(song.stream(), {
                    duration: songMeta.attributes.durationInMillis / 1000,
                    title: songMeta.attributes.name,
                    performer: songMeta.attributes.artistName,
                    fileMime: 'audio/mp4',
                    fileSize: song.size,
                    fileName: `${songMeta.attributes.name} - ${songMeta.attributes.artistName}.m4a`,
                }),
                {
                    progressCallback: async (
                        uploaded: number,
                        total: number
                    ) => {
                        const now = Date.now();
                        // Send an update only if 2 seconds have passed since the last update
                        if (now - lastUpdate >= 2000) {
                            lastUpdate = now;
                            const progressText = `${uploaded}/${total}`;

                            try {
                                tg.editMessage({
                                    message: uploadMessage.id,
                                    chatId: msg.chat.id,
                                    text: `Uploading song... ${progressText}`,
                                });
                            } catch (error) {
                                console.error(
                                    'Error sending progress update:',
                                    error
                                );
                            }
                        }
                    },
                }
            );

            await tg.deleteMessagesById(msg.chat.id, [uploadMessage.id]);
            if (songMsg.media?.type === 'audio') {
                msg.replyMedia(InputMedia.auto(songMsg.media.fileId));
                await File.addFile(
                    Number(urlMeta.id),
                    FileTypeEnum.SONG,
                    songMsg.media.fileId
                );
            }

            await unlink(filePath);
        }
    } catch (e) {
        console.log(e);
        const err = e instanceof Error ? e.message : 'Unknown error';
        await msg.answerText(err);
    }
}

export async function getSongMeta(
    urlMeta: URLMeta,
    token: string
): Promise<AutoSong | null> {
    const url = `https://amp-api.music.apple.com/v1/catalog/${urlMeta.storefront}/${urlMeta.urlType}/${urlMeta.id}`;

    const queryParams = new URLSearchParams({
        include: 'albums,explicit',
        extend: 'extendedAssetUrls',
        l: '',
    });

    const response = await fetch(`${url}?${queryParams.toString()}`, {
        method: 'GET',
        headers: {
            Authorization: `Bearer ${token}`,
            'User-Agent':
                'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36',
            Origin: 'https://music.apple.com',
        },
    });

    if (!response.ok) {
        throw new Error(`HTTP error! Status: ${response.status}`);
    }

    const songResponse: SongResponse = await response.json();

    for (const song of songResponse.data) {
        if (song.id === urlMeta.id) {
            return song;
        }
    }

    return null; // No matching song found
}
