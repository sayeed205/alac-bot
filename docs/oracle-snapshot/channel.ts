export class BoundedChannel<T> {
  private queue: T[] = []
  private waitingWriters: (() => void)[] = []
  private waitingReaders: ((item: T | undefined) => void)[] = []
  private isClosed = false

  constructor(
    public readonly capacity: number,
    private signal?: AbortSignal,
  ) {
    if (capacity < 1) {
      throw new Error('Channel capacity must be at least 1')
    }
  }

  async push(item: T): Promise<void> {
    if (this.signal?.aborted || this.isClosed) {
      throw new Error('Download was cancelled')
    }

    const reader = this.waitingReaders.shift()
    if (reader) {
      reader(item)
      return
    }

    if (this.queue.length < this.capacity) {
      this.queue.push(item)
      return
    }

    await new Promise<void>((resolve, reject) => {
      let onAbort: (() => void) | undefined

      const onSpace = () => {
        if (onAbort && this.signal) {
          this.signal.removeEventListener('abort', onAbort)
        }
        this.queue.push(item)
        resolve()
      }

      if (this.signal) {
        onAbort = () => {
          const idx = this.waitingWriters.indexOf(onSpace)
          if (idx !== -1) this.waitingWriters.splice(idx, 1)
          reject(new Error('Download was cancelled'))
        }
        this.signal.addEventListener('abort', onAbort, { once: true })
      }

      this.waitingWriters.push(onSpace)
    })
  }

  async pull(): Promise<T | undefined> {
    if (this.signal?.aborted) {
      return undefined
    }

    if (this.queue.length > 0) {
      const item = this.queue.shift()
      if (item !== undefined) {
        const writer = this.waitingWriters.shift()
        if (writer) {
          writer()
        }
        return item
      }
    }

    if (this.isClosed) {
      return undefined
    }

    return new Promise<T | undefined>((resolve) => {
      let onAbort: (() => void) | undefined

      const onItem = (item: T | undefined) => {
        if (onAbort && this.signal) {
          this.signal.removeEventListener('abort', onAbort)
        }
        resolve(item)
      }

      if (this.signal) {
        onAbort = () => {
          const idx = this.waitingReaders.indexOf(onItem)
          if (idx !== -1) this.waitingReaders.splice(idx, 1)
          resolve(undefined)
        }
        this.signal.addEventListener('abort', onAbort, { once: true })
      }

      this.waitingReaders.push(onItem)
    })
  }

  async *[Symbol.asyncIterator](): AsyncIterator<T> {
    while (true) {
      const item = await this.pull()
      if (item === undefined) break
      yield item
    }
  }

  close(): void {
    this.isClosed = true
    while (this.waitingReaders.length > 0) {
      const reader = this.waitingReaders.shift()
      if (reader) {
        reader(undefined)
      }
    }
  }

  drain(): T[] {
    return this.queue.splice(0, this.queue.length)
  }

  get size(): number {
    return this.queue.length
  }
}
