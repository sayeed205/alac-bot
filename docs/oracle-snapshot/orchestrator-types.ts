import type { TelegramClient } from '@mtcute/bun'

import type { IAlacService } from '@/modules/alac/service.ts'
import type { ISettingsService } from '@/modules/settings/service.ts'

import type { ParsedTargetItem } from '../parser.ts'
import type { IRipQueue } from '../queue.ts'
import type { ITrackRipper } from '../ripper.ts'

export interface ActiveRipJob {
  id: string
  chatId: number
  userId: number
  userName?: string
  jobHeader: string
  totalTracks: number
  statusMsgId: number
  controller: AbortController
  isCancelled: boolean
  cancelledBy?: string
  cachedCount: number
  rippedCount: number
  failedCount: number
  completed: boolean
  queuePosition?: number
  startTime: number
  activeActionText?: string
}

export interface RipJobOptions {
  chatId: number
  userId: number
  userName?: string
  deliveryChatId: number | string
  isGroup: boolean
  isForce: boolean
  isCacheOnly: boolean
  singleStorefront?: string
  parsedItems: ParsedTargetItem[]
  replyToMessageId?: number
  statusMsgId: number
  isAdmin: boolean
}

export interface RipJobProgress {
  jobId: string
  totalTracks: number
  completedTracks: number
  cachedCount: number
  rippedCount: number
  failedCount: number
  skippedCount: number
  percent: number
  activeDownloadText?: string
  activeUploadText?: string
  activityOverride?: string
}

export interface RipJobSummary {
  jobId: string
  jobHeader: string
  totalTracks: number
  cachedCount: number
  rippedCount: number
  failedCount: number
  failedTracks: { id: string; error: string }[]
  skippedUncachedTracks: string[]
  totalElapsedSec: string
  cappedCount: number
  maxCollectionLimit: number
  isCacheOnly: boolean
  isGroup: boolean
}

export interface OrchestratorDependencies {
  tg: TelegramClient
  service: IAlacService
  ripper: ITrackRipper
  queue: IRipQueue
  settings?: ISettingsService
  uploadRetryBaseMs?: number
}
