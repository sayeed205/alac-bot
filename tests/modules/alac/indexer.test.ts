import { describe, expect, it, mock } from 'bun:test'

import type { TelegramClient } from '@mtcute/bun'

import {
  formatDumpCaption,
  formatIndexSummaryHtml,
  indexDumpChannel,
  parseDumpCaption,
} from '@/modules/alac/indexer.ts'
import type { IAlacService, SaveTrackInput } from '@/modules/alac/service.ts'

describe('Dump Channel Indexer & Metadata Tagging', () => {
  it('formats dump caption with human-readable specs and machine-readable spoiler', () => {
    const caption = formatDumpCaption({
      appleTrackId: '1559523359',
      title: 'Never Gonna Give You Up',
      artist: 'Rick Astley',
      album: 'Whenever You Need Somebody',
      duration: 212,
      bitDepth: 24,
      sampleRate: 96000,
    })

    expect(caption.text).toContain('Never Gonna Give You Up')
    expect(caption.text).toContain('Rick Astley')
    expect(caption.text).toContain('Whenever You Need Somebody')
    expect(caption.text).toContain('24-bit • 96.0 kHz • 3:32')
    expect(caption.text).toContain('#alac:{"id":"1559523359"')

    // Verify spoiler entity is present
    const spoiler = caption.entities?.find(
      (e: { _: string }) => e._ === 'messageEntitySpoiler',
    )
    expect(spoiler).toBeDefined()
  })

  it('correctly parses dump caption with valid spoiler payload', () => {
    const text =
      '🎵 Never Gonna Give You Up — Rick Astley\n💽 Whenever You Need Somebody\n🎧 ALAC • 24-bit • 96.0 kHz\n\n#alac:{"id":"1559523359","album":"Whenever You Need Somebody","bit":24,"hz":96000}'

    const parsed = parseDumpCaption(text)
    expect(parsed).toEqual({
      appleTrackId: '1559523359',
      album: 'Whenever You Need Somebody',
      bitDepth: 24,
      sampleRate: 96000,
    })
  })

  it('returns null when parsing text without #alac payload', () => {
    expect(parseDumpCaption(null)).toBeNull()
    expect(parseDumpCaption('')).toBeNull()
    expect(
      parseDumpCaption('Random text without any metadata payload'),
    ).toBeNull()
    expect(parseDumpCaption('#alac:{corrupted_json}')).toBeNull()
    expect(parseDumpCaption('#alac:{"no_id": true}')).toBeNull()
  })

  it('indexes dump channel, saves valid tracks, and prunes deleted tracks', async () => {
    const savedTracks: SaveTrackInput[] = []
    let prunedNotIn: string[] = []

    const mockService = {
      saveTrack: mock((input: SaveTrackInput) => {
        savedTracks.push(input)
        return Promise.resolve({} as never)
      }),
      deleteTracksNotIn: mock((validIds: string[]) => {
        prunedNotIn = validIds
        return Promise.resolve(2) // simulate 2 ghost tracks deleted
      }),
    } as unknown as IAlacService

    // Create fake messages yielded by getMessages
    const fakeMessages = [
      // 1. Valid ALAC audio with metadata
      {
        id: 101,
        text: '🎵 Song 1\n#alac:{"id":"track_1","album":"Album 1","bit":24,"hz":48000}',
        media: {
          type: 'audio',
          fileId: 'file_id_1',
          uniqueFileId: 'uniq_1',
          title: 'Song 1',
          performer: 'Artist 1',
          duration: 180,
        },
      },
      // 2. Non-audio media (should be skipped)
      {
        id: 102,
        text: 'A photo message',
        media: {
          type: 'photo',
        },
      },
      // 3. Audio without #alac caption (legacy/unparseable, should be skipped)
      {
        id: 103,
        text: 'Just an audio file with no caption',
        media: {
          type: 'audio',
          fileId: 'file_id_3',
          title: 'Unknown Song',
        },
      },
      // 4. Valid second ALAC audio
      {
        id: 104,
        text: '🎵 Song 2\n#alac:{"id":"track_2","album":"Album 2","bit":16,"hz":44100}',
        media: {
          type: 'audio',
          fileId: 'file_id_2',
          uniqueFileId: 'uniq_2',
          title: 'Song 2',
          performer: 'Artist 2',
          duration: 200,
        },
      },
    ]

    const fakeTg = {
      sendText: mock(() => Promise.resolve({ id: 105 })),
      deleteMessagesById: mock(() => Promise.resolve()),
      getMessages: mock((_chatId: number | string, ids: number[]) => {
        const found = fakeMessages.filter((m) => ids.includes(m.id))
        return Promise.resolve(found)
      }),
    } as unknown as TelegramClient

    const progressUpdates: Array<{ scanned: number; synced: number }> = []

    const summary = await indexDumpChannel(
      fakeTg,
      mockService,
      -100123456,
      (scanned, synced) => {
        progressUpdates.push({ scanned, synced })
      },
    )

    expect(summary.scanned).toBe(4)
    expect(summary.synced).toBe(2)
    expect(summary.skipped).toBe(2)
    expect(summary.pruned).toBe(2)

    expect(savedTracks.length).toBe(2)
    // Saved in reverse order because we iterate from highest ID (104) down to lowest (101)
    expect(savedTracks).toContainEqual({
      appleTrackId: 'track_1',
      messageId: 101,
      fileId: 'file_id_1',
      fileUniqueId: 'uniq_1',
      title: 'Song 1',
      artist: 'Artist 1',
      album: 'Album 1',
      duration: 180,
      bitDepth: 24,
      sampleRate: 48000,
    })
    expect(savedTracks).toContainEqual({
      appleTrackId: 'track_2',
      messageId: 104,
      fileId: 'file_id_2',
      fileUniqueId: 'uniq_2',
      title: 'Song 2',
      artist: 'Artist 2',
      album: 'Album 2',
      duration: 200,
      bitDepth: 16,
      sampleRate: 44100,
    })

    expect(prunedNotIn).toContain('track_1')
    expect(prunedNotIn).toContain('track_2')
  })

  it('formats index completion summary to clean HTML', () => {
    const htmlSummary = formatIndexSummaryHtml({
      scanned: 50,
      synced: 45,
      pruned: 3,
      skipped: 5,
      durationMs: 2500,
    })

    expect(htmlSummary.text).toContain('Dump Channel Sync Complete')
    expect(htmlSummary.text).toContain('50')
    expect(htmlSummary.text).toContain('45')
    expect(htmlSummary.text).toContain('3')
    expect(htmlSummary.text).toContain('5')
    expect(htmlSummary.text).toContain('2.5s')
  })
})
