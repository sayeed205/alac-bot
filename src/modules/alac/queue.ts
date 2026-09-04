export interface EnqueueOptions {
  onPositionChange?: (position: number) => void
  onStart?: () => void
}

interface QueuedItem<T> {
  id: number
  task: (signal: AbortSignal) => Promise<T>
  resolve: (value: T | PromiseLike<T>) => void
  reject: (reason?: unknown) => void
  controller: AbortController
  options?: EnqueueOptions
}

export interface IRipQueue {
  enqueue<T>(
    task: (signal: AbortSignal) => Promise<T>,
    options?: EnqueueOptions,
  ): Promise<T>
  getPendingCount(): number
  isProcessing(): boolean
  clear(): void
}

export class SequentialRipQueue implements IRipQueue {
  private queue: QueuedItem<unknown>[] = []
  private activeItem: QueuedItem<unknown> | null = null
  private nextId = 1

  getPendingCount(): number {
    return this.queue.length
  }

  isProcessing(): boolean {
    return this.activeItem !== null
  }

  clear(): void {
    for (const item of this.queue) {
      item.controller.abort()
      item.reject(new Error('Queue cleared'))
    }
    this.queue = []
  }

  enqueue<T>(
    task: (signal: AbortSignal) => Promise<T>,
    options?: EnqueueOptions,
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const controller = new AbortController()
      const item: QueuedItem<T> = {
        id: this.nextId++,
        task,
        resolve: resolve as (value: unknown) => void,
        reject,
        controller,
        options,
      }

      this.queue.push(item as QueuedItem<unknown>)

      // If nothing is actively processing, start immediately
      if (!this.activeItem) {
        this.processNext()
      } else {
        // Otherwise notify position in queue (1-indexed)
        options?.onPositionChange?.(this.queue.length)
      }
    })
  }

  private async processNext(): Promise<void> {
    if (this.queue.length === 0) {
      this.activeItem = null
      return
    }

    const next = this.queue.shift()
    if (!next) {
      this.activeItem = null
      return
    }

    this.activeItem = next

    // Notify remaining queue items of their updated positions
    for (let i = 0; i < this.queue.length; i++) {
      this.queue[i]?.options?.onPositionChange?.(i + 1)
    }

    try {
      this.activeItem.options?.onStart?.()
      const result = await this.activeItem.task(
        this.activeItem.controller.signal,
      )
      this.activeItem.resolve(result)
    } catch (err) {
      this.activeItem.reject(err)
    } finally {
      this.activeItem = null
      await this.processNext()
    }
  }
}

export const ripQueue = new SequentialRipQueue()
