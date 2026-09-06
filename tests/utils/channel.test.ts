import { describe, expect, it } from 'bun:test'

import { BoundedChannel } from '@/utils/channel.ts'

describe('BoundedChannel', () => {
  it('pushes and pulls items within capacity', async () => {
    const channel = new BoundedChannel<number>(2)
    await channel.push(1)
    await channel.push(2)

    expect(channel.size).toBe(2)
    expect(await channel.pull()).toBe(1)
    expect(await channel.pull()).toBe(2)
    expect(channel.size).toBe(0)
  })

  it('blocks writer when capacity is reached until a reader pulls', async () => {
    const channel = new BoundedChannel<string>(2)
    await channel.push('a')
    await channel.push('b')

    let pushed = false
    const pushPromise = channel.push('c').then(() => {
      pushed = true
    })

    // Capacity is 2, so pushing 'c' should wait
    await new Promise((r) => setTimeout(r, 15))
    expect(pushed).toBe(false)

    // Reader pulls 'a'
    const first = await channel.pull()
    expect(first).toBe('a')

    // Writer unblocks
    await pushPromise
    expect(pushed).toBe(true)

    expect(await channel.pull()).toBe('b')
    expect(await channel.pull()).toBe('c')
  })

  it('blocks reader when empty until a writer pushes', async () => {
    const channel = new BoundedChannel<number>(2)

    let pulledValue: number | undefined
    const pullPromise = channel.pull().then((val) => {
      pulledValue = val
    })

    await new Promise((r) => setTimeout(r, 15))
    expect(pulledValue).toBeUndefined()

    await channel.push(42)
    await pullPromise
    expect(pulledValue).toBe(42)
  })

  it('resolves readers with undefined when closed', async () => {
    const channel = new BoundedChannel<number>(2)
    await channel.push(10)
    channel.close()

    expect(await channel.pull()).toBe(10)
    expect(await channel.pull()).toBeUndefined()
  })

  it('aborts waiting writers on AbortSignal', async () => {
    const controller = new AbortController()
    const channel = new BoundedChannel<number>(1, controller.signal)

    await channel.push(1)

    const pushPromise = channel.push(2)
    controller.abort()

    expect(pushPromise).rejects.toThrow('Download was cancelled')
  })

  it('resolves waiting readers with undefined on AbortSignal', async () => {
    const controller = new AbortController()
    const channel = new BoundedChannel<number>(1, controller.signal)

    const pullPromise = channel.pull()
    controller.abort()

    expect(await pullPromise).toBeUndefined()
  })

  it('drains remaining items cleanly', async () => {
    const channel = new BoundedChannel<number>(3)
    await channel.push(1)
    await channel.push(2)

    const drained = channel.drain()
    expect(drained).toEqual([1, 2])
    expect(channel.size).toBe(0)
  })
})
