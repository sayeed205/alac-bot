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
  it('formats dump caption with human-readable specs and machine-readable expandable blockquote', () => {
    const caption = formatDumpCaption({
      appleTrackId: '1559523359',
      title: 'Never Gonna Give You Up',
      artist: 'Rick Astley',
      album: 'Whenever You Need Somebody',
      duration: 212,
      bitDepth: 24,
      sampleRate: 96000,
      genre: 'Pop',
      releaseDate: '1987-11-12',
      trackNumber: 1,
      trackCount: 10,
    })

    expect(caption.text).toContain('Never Gonna Give You Up')
    expect(caption.text).toContain('Rick Astley')
    expect(caption.text).toContain('Whenever You Need Somebody')
    expect(caption.text).toContain('"id": "1559523359"')

    const blockquote = caption.entities?.find(
      (e: { _: string; collapsed?: boolean }) =>
        e._ === 'messageEntityBlockquote' && e.collapsed === true,
    )
    expect(blockquote).toBeDefined()
    // Verify JSON payload is contained within caption text
    expect(caption.text).toContain('"id": "1559523359"')
  })

  it('correctly parses dump caption with valid metadata payload', () => {
    const text =
      '🎵 Never Gonna Give You Up — Rick Astley\n💽 Whenever You Need Somebody\n🎧 ALAC • 24-bit • 96.0 kHz\n\n{\n  "id": "1559523359",\n  "title": "Never Gonna Give You Up",\n  "artist": "Rick Astley",\n  "album": "Whenever You Need Somebody",\n  "bit": 24,\n  "hz": 96000,\n  "dur": 212,\n  "genre": "Pop",\n  "date": "1987-11-12",\n  "trk": 1,\n  "cnt": 10\n}'

    const parsed = parseDumpCaption(text)
    expect(parsed).toEqual({
      appleTrackId: '1559523359',
      title: 'Never Gonna Give You Up',
      artist: 'Rick Astley',
      album: 'Whenever You Need Somebody',
      bitDepth: 24,
      sampleRate: 96000,
      duration: 212,
      genre: 'Pop',
      releaseDate: '1987-11-12',
      trackNumber: 1,
      trackCount: 10,
    })
  })

  it('returns null when parsing text without json payload', () => {
    expect(parseDumpCaption(null)).toBeNull()
    expect(parseDumpCaption('')).toBeNull()
    expect(
      parseDumpCaption('Random text without any metadata payload'),
    ).toBeNull()
    expect(parseDumpCaption('{corrupted_json}')).toBeNull()
    expect(parseDumpCaption('{"no_id": true}')).toBeNull()
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
        return Promise.resolve(2)
      }),
    } as unknown as IAlacService

    const fakeMessages = [
      {
        id: 101,
        text: '🎵 Song 1\n<blockquote expandable>{\n  "id": "track_1",\n  "title": "Song 1 Full",\n  "artist": "Artist 1 Full",\n  "album": "Album 1",\n  "bit": 24,\n  "hz": 48000,\n  "dur": 185,\n  "genre": "Rock",\n  "date": "2021-01-01",\n  "trk": 2,\n  "cnt": 12\n}</blockquote>',
        media: {
          type: 'audio',
          fileId: 'file_id_1',
          uniqueFileId: 'uniq_1',
          title: 'Song 1',
          performer: 'Artist 1',
          duration: 180,
        },
      },
      {
        id: 102,
        text: 'A photo message',
        media: {
          type: 'photo',
        },
      },
      {
        id: 103,
        text: 'Just an audio file with no caption',
        media: {
          type: 'audio',
          fileId: 'file_id_3',
          title: 'Unknown Song',
        },
      },
      {
        id: 104,
        text: '🎵 Song 2\n<blockquote expandable>{\n  "id": "track_2",\n  "album": "Album 2",\n  "bit": 16,\n  "hz": 44100\n}</blockquote>',
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
    expect(savedTracks).toContainEqual({
      appleTrackId: 'track_1',
      messageId: 101,
      fileId: 'file_id_1',
      fileUniqueId: 'uniq_1',
      title: 'Song 1 Full',
      artist: 'Artist 1 Full',
      album: 'Album 1',
      duration: 185,
      bitDepth: 24,
      sampleRate: 48000,
      genre: 'Rock',
      releaseDate: '2021-01-01',
      trackNumber: 2,
      trackCount: 12,
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
      genre: 'Music',
      releaseDate: '',
      trackNumber: 1,
      trackCount: 1,
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
