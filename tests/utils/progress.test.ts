import { describe, expect, it } from 'bun:test'

import {
  formatByteProgress,
  formatBytes,
  formatMbProgress,
  renderProgressBar,
} from '@/utils/progress.ts'

describe('Progress Bar Utility', () => {
  it('renders 0% progress bar correctly', () => {
    expect(renderProgressBar(0, 100, 10)).toBe('[░░░░░░░░░░] 0%')
  })

  it('renders 50% progress bar correctly', () => {
    expect(renderProgressBar(50, 100, 10)).toBe('[█████░░░░░] 50%')
  })

  it('renders 100% progress bar correctly', () => {
    expect(renderProgressBar(100, 100, 10)).toBe('[██████████] 100%')
  })

  it('clamps values above 100% to 100%', () => {
    expect(renderProgressBar(150, 100, 10)).toBe('[██████████] 100%')
  })

  it('handles 0 or negative total gracefully', () => {
    expect(renderProgressBar(50, 0, 10)).toBe('[░░░░░░░░░░] 0%')
    expect(renderProgressBar(50, -10, 10)).toBe('[░░░░░░░░░░] 0%')
  })

  it('formats byte progress with MB values', () => {
    const current = 15 * 1024 * 1024
    const total = 30 * 1024 * 1024
    const formatted = formatByteProgress(current, total, 10)
    expect(formatted).toBe('[█████░░░░░] 50% (15.0/30.0 MB)')
  })

  it('formats MB progress without progress bar', () => {
    const current = 18.2 * 1024 * 1024
    const total = 45.1 * 1024 * 1024
    expect(formatMbProgress(current, total)).toBe('18.2/45.1 MB')
    expect(formatMbProgress(current, 0)).toBe('18.2 MB')
  })

  it('formats bytes correctly', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1024)).toBe('1.00 KB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.00 MB')
    expect(formatBytes(2.5 * 1024 * 1024 * 1024)).toBe('2.50 GB')
  })
})
