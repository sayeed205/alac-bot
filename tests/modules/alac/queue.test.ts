import { describe, expect, it } from 'bun:test'

import { SequentialRipQueue } from '@/modules/alac/queue.ts'

describe('SequentialRipQueue', () => {
  it('executes tasks sequentially one at a time', async () => {
    const queue = new SequentialRipQueue()
    const executionOrder: number[] = []

    const task1 = queue.enqueue(async () => {
      await new Promise((r) => setTimeout(r, 20))
      executionOrder.push(1)
      return 'one'
    })

    const task2 = queue.enqueue(async () => {
      await new Promise((r) => setTimeout(r, 10))
      executionOrder.push(2)
      return 'two'
    })

    const [res1, res2] = await Promise.all([task1, task2])

    expect(res1).toBe('one')
    expect(res2).toBe('two')
    expect(executionOrder).toEqual([1, 2])
    expect(queue.isProcessing()).toBe(false)
    expect(queue.getPendingCount()).toBe(0)
  })

  it('tracks queue position changes correctly as tasks advance', async () => {
    const queue = new SequentialRipQueue()
    const task2Positions: number[] = []
    const task3Positions: number[] = []

    let task2Start = false

    const task1 = queue.enqueue(async () => {
      await new Promise((r) => setTimeout(r, 20))
      return 1
    })

    const task2 = queue.enqueue(
      async () => {
        await new Promise((r) => setTimeout(r, 20))
        return 2
      },
      {
        onPositionChange: (pos) => task2Positions.push(pos),
        onStart: () => {
          task2Start = true
        },
      },
    )

    const task3 = queue.enqueue(
      async () => {
        return 3
      },
      {
        onPositionChange: (pos) => task3Positions.push(pos),
      },
    )

    expect(task2Positions).toEqual([1])
    expect(task3Positions).toEqual([2])

    await task1
    expect(task2Start).toBe(true)
    expect(task3Positions).toContain(1)

    await Promise.all([task2, task3])
    expect(queue.isProcessing()).toBe(false)
  })

  it('continues processing remaining tasks even if one task throws an error', async () => {
    const queue = new SequentialRipQueue()

    const task1 = queue.enqueue(async () => {
      throw new Error('Rip failed')
    })

    const task2 = queue.enqueue(async () => {
      return 'success'
    })

    expect(task1).rejects.toThrow('Rip failed')
    expect(await task2).toBe('success')
  })
})
