import { describe, expect, it } from 'bun:test'

import {
  convertTtmlToElrc,
  detectLyricsTier,
  LyricsTier,
  scoreLyrics,
} from '@/modules/alac/lyrics.ts'

describe('Lyrics Ranking & Parser', () => {
  it('converts Apple Music TTML with word spans to Enhanced LRC', () => {
    const ttml = `
      <tt xmlns="http://www.w3.org/ns/ttml">
        <body>
          <div>
            <p begin="27.395" end="28.960">
              <span begin="27.395" end="27.549">I</span>
              <span begin="27.549" end="27.740">been</span>
              <span begin="27.740" end="28.077">tryna</span>
              <span begin="28.077" end="28.960">call</span>
            </p>
            <p begin="30.189" end="32.529">
              <span begin="30.189" end="30.396">I've</span>
              <span begin="30.396" end="30.642">been</span>
              <span begin="30.642" end="30.799">on</span>
              <span begin="30.799" end="30.984">my</span>
              <span begin="30.984" end="31.245">own</span>
            </p>
          </div>
        </body>
      </tt>
    `

    const elrc = convertTtmlToElrc(ttml)
    expect(elrc).not.toBeNull()
    const lines = elrc?.split('\n') ?? []
    expect(lines.length).toBe(2)
    expect(lines[0]).toBe(
      '[00:27.395]<00:27.395>I <00:27.549>been <00:27.740>tryna <00:28.077>call',
    )
    expect(lines[1]).toBe(
      "[00:30.189]<00:30.189>I've <00:30.396>been <00:30.642>on <00:30.799>my <00:30.984>own",
    )
  })

  it('detects WORD_SYNCED tier for Enhanced LRC', () => {
    const elrc =
      '[00:15.200]<00:15.200>Hello <00:15.800>world\n[00:18.000]<00:18.000>Second <00:18.400>line'
    expect(detectLyricsTier(elrc)).toBe(LyricsTier.WORD_SYNCED)
  })

  it('detects LINE_SYNCED tier for standard LRC', () => {
    const lrc = '[00:15.200]Hello world\n[00:18.000]Second line'
    expect(detectLyricsTier(lrc)).toBe(LyricsTier.LINE_SYNCED)
  })

  it('detects PLAIN tier for lyrics without timestamps', () => {
    const plain = 'Hello world\nThis is a plain lyrics text\nNo timestamps here'
    expect(detectLyricsTier(plain)).toBe(LyricsTier.PLAIN)
  })

  it('returns NONE for empty or single-word junk', () => {
    expect(detectLyricsTier('')).toBe(LyricsTier.NONE)
    expect(detectLyricsTier('Instrumental')).toBe(LyricsTier.NONE)
  })

  it('strictly prioritizes word-by-word sync over line-synced even with higher provider weight', () => {
    const lineSyncedFromPrimary = scoreLyrics(
      '[00:10.00]Line 1\n[00:15.00]Line 2',
      'Paxsenix (Primary)',
      30,
    )
    const wordSyncedFromSecondary = scoreLyrics(
      '[00:10.00]<00:10.00>Line <00:12.00>1\n[00:15.00]<00:15.00>Line <00:17.00>2',
      'BetterLyrics (Secondary)',
      20,
    )

    // Primary line-synced score = 500 + 30 = 530
    // Secondary word-synced score = 1000 + 20 = 1020
    expect(wordSyncedFromSecondary.score).toBeGreaterThan(
      lineSyncedFromPrimary.score,
    )
  })
})
