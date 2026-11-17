import type { RestoreStage, RestoreKind, RestoreFileResult } from './requests/restore-types';
export type { RestoreStage, RestoreKind, RestoreFileResult };
import type { SettingsRequest, SettingsSnapshot, SettingsApplyItem } from './requests/settings-compare';
import type { TransferRequest, TransferExportResult, TransferPreview, TransferApplyResult } from './requests/transfer';
import type { RemoteBridge, RemoteConnection, RemoteHostEntry } from '../remote/shared';
import type { TerminalBridge } from './terminal-bridge';
import type { BranchInput } from './branch-input';
import type { DelegationBridge } from './delegation-bridge';
import type { WindowChromeBridge } from './window-bridge';
import type { RecordsBridge } from './records-bridge';
export type { RecordsPhase, RecordsBackup } from './records-bridge';
import type { FileExport } from './file-export';
export type { TerminalBridge, TerminalScreen } from './terminal-bridge';
import type { AnswerLanguageSetting, AppliedAnswerLanguage, EngineLanguage } from './engine-text';
import type { SessionSync, SyncProbeResult, SyncRequest, SyncView } from './sync-contract';
import type { ShareExpiryDays } from './share-format';
import type { UiLanguage, UiLanguageSetting } from './i18n/core';
import type { WireText } from './i18n/wire';
import type { EngineModel, LocalProviderSetting, LocalProviderView } from './engine-model';
export type { EngineModel, EngineSpeed, LocalModelView, LocalModelWarn, LocalProviderKind, LocalProviderSetting, LocalProviderView } from './engine-model';
export { LOCAL_CONTEXT_WARN_TOKENS, LOCAL_PROVIDER_DEFAULTS, LOCAL_PROVIDER_KINDS } from './engine-model';
import type { Activity, Attachment, DocKindView, DocLocView, DocStateView, SessionRun, Steer, TurnCheckpoint } from './session-run';
export type { Attachment, AttachmentScene, DocKindView, DocLocView, DocStateName, DocStateView, Steer } from './session-run';
import type { MainCommandId, ScreenCommandId } from './commands';
import type { CaptureShortcutBridge, KeybindingsBridge } from './keybindings';
import type { RoleId, RolePolicy } from './role-policy';
import type { CopyApplyPreview, CopyChangeResult, CopyCleanPreview } from './isolated-copy-types';
import type { FileVersionList, FileVersionView, VersionImageSide } from './file-version-types';
import type { FilePreview } from './file-view-types';
export type { FileInfo, FilePreview, ThumbnailSource } from './file-view-types';
import type { ProjectFilesAction, ProjectFilesResult, ProjectTrashApproval } from './project-files-types';
import type { AttentionEvent, AttentionItem, AttentionTarget } from './attention';
import type { HostPickPurpose } from './guest-action';
export type { HostPickPurpose } from './guest-action';
export type { AttentionChecks, AttentionEvent, AttentionItem, AttentionTarget } from './attention';
export type { LairImportFolders, LairImportPreview, LairImportResult } from './lair-import-types';
import type { McpCallList, McpImportCandidates, McpLoginStart, McpServerConfig, McpView } from './mcp-types';
export type { McpAuthRecord, McpAuthState, McpAuthSummary, McpCallDecision, McpCallErrorKind, McpCallList, McpCallRecord, McpCallResult, McpCheck, McpEngineSet, McpField, McpImportCandidate, McpImportCandidates, McpLoginStart, McpOAuthConfig, McpOAuthOpenMessage, McpPolicy, McpPresetId, McpScope, McpSecretKeysMessage, McpSecretsMessage, McpSecretStoredMessage, McpSecretStoreMessage, McpServerConfig, McpServerInput, McpServerSummary, McpSource, McpToolInfo, McpToolRule, McpTransport, McpView } from './mcp-types';
import type { ExtIssueLink, ExtIssueResult, ExtIssuesResult, ExtIssueSource, GithubAction, GithubCheckLogResult, GithubChecksSummary, GithubIssueLink, GithubIssueResult, GithubIssuesResult, GithubPrCreateResult, GithubPrDraft, GithubPrRecord, GithubReadResult, GithubReviewsResult } from './github-types';
export type { ExtIssueComment, ExtIssueDetail, ExtIssueLink, ExtIssueResult, ExtIssuesResult, ExtIssueSource, ExtIssueSummary, GithubAction, GithubCheck, GithubCheckBucket, GithubCheckEvent, GithubCheckLogResult, GithubChecksCounts, GithubChecksSummary, GithubDraftIssue, GithubIssueDetail, GithubIssueLink, GithubIssueResult, GithubIssuesResult, GithubIssueSummary, GithubPrCreateExpect, GithubPrCreateRecord, GithubPrCreateResult, GithubPrCreateStage, GithubPrDraft, GithubPrDraftCommit, GithubPrRecord, GithubReadResult, GithubReview, GithubReviewComment, GithubReviewsResult, GithubReviewThread, GithubStatus, GithubVerifyItem } from './github-types';
import type { EngineInstallView, EngineLatestView, EngineLocateView, EngineSourceView, EngineUpdateNote, EngineUpdatesView } from './engine-setup-types';
export type { EngineInstallPhase, EngineInstallView, EngineLatestView, EngineLocateView, EngineSourceKind, EngineSourceView, EngineUpdateBatchItem, EngineUpdateBatchView, EngineUpdateNote, EngineUpdatesView } from './engine-setup-types';
import type { CommitDetail, FileHistory, HistoryList } from './git-history-types';
export type { CommitDetail, CommitFile, FileHistory, FileHistoryItem, HistoryCommit, HistoryList, HistoryRef } from './git-history-types';
import type { GitBadge, GitFailureCode, GitRemoteStatus } from './git-remote-types';
export type { GitBadge, GitFailureCode, GitRemoteStatus, PushTarget } from './git-remote-types';
import type { DelegateTargetInput, Delegation, DelegationApplyPreview, DelegationChangeResult, DelegationSteerText, SessionUsage, Usage, WorkerMode } from './delegation-types';
import type { SessionCancelFields, TurnCancelFields } from './send-cancel-types';
export type { SessionUsage, DelegateTargetInput, Delegation,DelegationActivity, DelegationActivitySummary, DelegationApplyOp, DelegationApplyPreview, DelegationAttempt, DelegationChangeResult, DelegationFileChange, DelegationHandoffEvent, DelegationMode, DelegationPriorUsage, DelegationProgress, DelegationSteerText, DelegationStatus, DelegationSupervisor, DelegationToolFailure, DelegationWrite, Usage, WorkerActivityKind, WorkerActivityState, WorkerMode } from './delegation-types';
import type { CommonMemoryAction, CommonMemoryResult, MemoryCandidate, TurnMemoryDelivery } from './common-memory-types';
import type { CommonRulesAction, CommonRulesView, MemoryAction, MemoryResult, RuleFileSave, RuleFileView } from './rules-types';
export type { CommonMemoryScope, CommonMemoryHow, CommonMemorySource, CommonMemory, CommonMemoryLogEntry, MemoryCandidate, TurnMemoryDelivery, MemoryCandidateView, CommonMemoryAction, CommonMemoryResult } from './common-memory-types';
import type { RecordScopeRequest, SessionReadBridge, SnapshotDiffFields, SnapshotKnown, TurnDiffFields } from './turn-diff';
import type { QuickSetupAnswers, SettingId, SettingRisk, SettingValue } from './settings-catalog';
import type { TurnBrowser } from './preview-agent';
import type { PreviewBrowserBridgeBase, RemoteBrowserRect, RemoteBrowserRequest } from './preview-bridge';
import type { BrowserOriginsRequest, BrowserOriginsResult } from './browser-origins';
import type { CommandAllow, CommandAllowList, CommandAllowRequest } from './requests/command-allows';
import type { ProjectPlaceRequest, RelocatePlan, SessionFolder } from './requests/projects';
import type { ConversationTagRequest } from './requests/conversation-tags';
import type { McpMentionableResult, McpRequest } from './requests/mcp';
import type { ProjectGroupFields, ProjectGroupRequest } from './requests/project-groups';
import type { AttachmentClipRequest } from './requests/attachment-clips';
import type { ConversationTag } from './conversation-color';
import type { IssueSourcesView } from './issue-source-types';
export type { IssueSourcesConfig, IssueSourcesView, JiraSourceConfig, LinearSourceConfig } from './issue-source-types';
import type { CloseBehavior, CloseMode, UiThemeBridge } from './ui-theme-types';
import type { FolderScopeInfo, FolderStatus } from './requests/project-folders';
export type { FolderStatus } from './requests/project-folders';
import type { PaneSetRequest, PaneSetResult } from './requests/pane-sets';
export type { FolderPlace, SessionFolder } from './requests/projects';
import type { Project } from './project';
export type { Project, ProjectFolder } from './project';
export { PROJECT_FOLDER_LIMIT } from './project';
import type { SessionApprovalRequest } from './requests/session-approval';
import type { ReportToParentResult, SessionParentRequest } from './requests/session-parent';
import type { EngineRequest } from './requests/engine-actions';
import type { ScheduleRequest } from './requests/schedules';
import type { BudgetRef, BudgetSize, BudgetUnit, RequestBudget } from './request-budget';
import type { BudgetRequest } from './requests/budget';
import type { ShareDraft, ShareItem, ShareList } from './requests/shares';
import type { CodeCompleteResult, CodeCompletionFormat, CodeCompletionSetting, CodeCompletionSource, CodeCompletionView, ProviderModelsResult } from './requests/code-completion';
import type { ReplySummaryRequest, ReplySummaryResult } from './requests/reply-summary';
import type { ReleaseNotesRequest, ReleaseNotesView } from './requests/release-notes';
import type { AccountCreditRequest } from './requests/account-credits';
import type { AnswerDetailFields, AnswerDetailRequest } from './requests/answer-detail';
import type { AppSettingRequest } from './requests/view-options';
import type { WorktreeReadRequest } from './requests/worktrees';
import type { NewChatDefaultSnapshot, SetNewChatDefaultRequest } from './new-chat-default';
export type { NewChatDefaultSnapshot, SetNewChatDefaultRequest } from './new-chat-default';
import type { AppSettingSnapshot } from './app-setting-snapshot';
import type { SetEngineEnabledRequest } from './engine-switch';
import type { PreparedChangeResult, RequestReceiptState, SessionRunRequest } from './requests/session-run';
export type { PreparedChangeItem, PreparedChangeResult, RequestReceiptState } from './requests/session-run';
import type { MultiSendRequest, MultiSendResult, StagedFile } from './requests/multi-send';
export type { MultiSendResult, MultiSendTarget, MultiSendTargetResult, StagedFile } from './requests/multi-send';
import type { CliOverview, CliOverviewRequest } from './cli-door';
import type { FindResult, SearchResult } from './session-search';
export type { FindResult, SearchHit, SearchResult, SearchSession, SearchWhere } from './session-search';
import type { ApprovalMode, EngineId, AcpEngineId, SentFrom, SentFromRoot } from './core-types';
import type { ScheduleFinishResult, ScheduleListResult, SchedulePrecheck, ScheduleView } from './schedules';
import type { AnsweredFrom, Approval, RespondApprovalRequest } from './approval';
export type { AppActionPlanCard, AppActionPlanStep, Approval, ApprovalQuestion, ElicitationField, ElicitationRequest, ElicitationValue } from './approval';
import type { UpdateStatus } from './app-update';
export type { UpdateResult, UpdateStatus } from './app-update';
import type { EngineThread } from './engine-thread';
import type { Handoff, Compaction } from './handoff-record';
import type { ImportedFoldersInspection, ImportedFoldersMapResult, ChatFolderProjectsInspection } from './folder-results';
import type { EngineImportListResult, EngineImportOpenResult, EngineImportRequest, EngineImportSourcesResult, ExternalThread, ImportedMark } from './engine-import';
import type { MigrateRequest, MigrateResult } from './requests/migrate';
export type TurnStatus = 'running' | 'waiting-approval' | 'stopping' | 'completed' | 'failed' | 'interrupted' | 'unknown';

export type { ApprovalMode } from './core-types';
import type { ChildResult, SessionParent, SubSessionChild } from './sub-session-types';
import type { WorktreeStage, WorktreeMergeMethod, WorktreeMergeStep, WorktreeMerged, WorktreeSetup, WorktreeCleanPreview, WorktreeMergeResult, WorktreeIncludePreview } from './worktree-types';
import type { SpeechLanguage, SpeechRunnerChoice, SpeechStatus } from './speech-types';
export type { SpeechLanguage, SpeechRunnerChoice, SpeechStatus } from './speech-types';
import type { WebConnectionState } from './web-connection-types';
export type { WebConnectionState } from './web-connection-types';
import type { UiZoomBridge } from './ui-zoom';
export type { ChildResult, ChildResultKind, ChildResultView, SessionParent, SubSessionChild } from './sub-session-types';

import type { QueuedRequest, RequestMode } from './queued-request';
import type { ParticipantKeyProvider, TurnExecution } from './turn-execution';
import type { TurnEvidence, WebSiteControl } from './evidence-types';
export type { QueuedRequest, RequestMode } from './queued-request';
export const REQUEST_MODES: readonly RequestMode[] = ['research', 'plan', 'execute'];

export interface ResultFile { path: string; source: 'changed' | 'mentioned' }
import type { Note, NoteSummary, NoteSearchHit, NoteChange } from './requests/note-model';
export { NOTE_MAX_CHARS } from './requests/note-model';
export type { Note, NoteSummary, NoteSearchHit, NoteChange } from './requests/note-model';
/**
 * 캡처 보드(docs/notes/capture-board-2026-10-14.md 3절·구현 계약). 앱 공용 보드와 카드의 확정 상태는 실행부가 소유한다.
 * 그림 파일은 <앱 데이터>/boards/<카드 ID>/<version>-original.png·<version>-thumb.webp 에 있고 목록·읽기에는 바이트를 싣지 않는다.
 */
export const BOARD_MAX_CARDS = 500;
export const BOARD_MAX_BOARDS = 50;
export const BOARD_TEXT_MAX_CHARS = 20_000;
export const BOARD_IMAGE_MAX_BYTES = 40 * 1024 * 1024;
export const BOARD_IMAGE_MAX_SIDE = 16_384;
export const BOARD_IMAGE_MAX_PIXELS = 40_000_000;
export const BOARD_THUMB_SIDE = 768;
export const BOARD_THUMB_MAX_BYTES = 2 * 1024 * 1024;
import type { Board, BoardCard, BoardChange, BoardClipBridge, BoardDragBridge, BoardImageInput, BoardMoveResult, BoardPlacement, BoardScreenCaptureKind, BoardScreenCaptureResult, BoardSummary, BoardTextInput, BoardWindowList, ScreenCapturePermission } from './board-types';
export type { Board, BoardCard, BoardCardKind, BoardCardSource, BoardChange, BoardImageInput, BoardMoveResult, BoardPlacement, BoardScreenCaptureKind, BoardScreenCaptureResult, BoardSummary, BoardTextInput, BoardWindowList, ScreenCapturePermission } from './board-types';
export interface BoardAttachResult { snapshot: Snapshot | null; added: string[]; skipped: { cardId: string; reason: 'limit' | 'too-large' | 'missing' }[]; clips?: { cardId: string; durationMs: number; total: number; frames: { name: string; ms: number; kind: import('./board-video').BoardClipFrameKind }[] }[] }
export interface Draft { text: string; revision: number; /** 임시 대화의 초안. 원격으로 보내지 않는다(docs/notes/temporary-chat-2026-10-03.md T10). */ temporary?: true }
/** loose는 기록 없는 미사용 사본이다. 실행부가 열 때 그 자리에서 사본을 확인한다(docs/notes/unused-cleanup-2026-09-26.md 2번). */
export type FileTarget = { sessionId: string; turnId?: string; attachmentId: string; loose?: boolean }
  | { sessionId: string; turnId: string; resultPath: string }
  /** Codex 가 만든 그림(turn.generated 에 있는 것만, 열기 루트는 그 Codex 홈의 generated_images). docs/notes/image-results-2026-10-03.md D2 */
  | { sessionId: string; turnId: string; generatedPath: string }
  /** 답 안 로컬 그림(미리보기·썸네일만. 같은 문서 D5) */
  | { sessionId: string; turnId: string; replyImage: string }
  /** 답·질문 카드에 적힌 로컬 파일 경로(미리보기·사본 저장, 첨부는 안 됨). 그 요청의 글에 그대로 있어야 연다. docs/notes/b79-reply-paths-2026-10-12.md P4, b101-reply-file-save-2026-10-14.md D1. delegationId 가 있으면 그 위임의 결과·부분 답 글과 위임 작업 폴더 기준(B-330) */
  | { sessionId: string; turnId: string; replyPath: string; delegationId?: string }
  | { workspacePath: string }
  /** 프로젝트 자료(사본은 project-sources 안, 참조는 프로젝트 폴더 안 원본). 실행부가 번호로 저장 경로를 푼다(docs/notes/documents-2026-10-03.md D6·D8). */
  | { projectId: string; sourceId: string };
/**
 * 크게 보기 대화 이미지 목록의 한 장(docs/notes/image-viewer-2026-10-13.md 4.7절 1번). 그림 바이트는 싣지 않고, 화면이 target 으로 previewFile 을 불러 원본을 받는다.
 * key 는 대화 안에서 유일하다: 화면 panels.ts targetKey(target) 과 같은 꼴에서 파일 대상이 가리키는 값(첨부 `a:대화:첨부`, 생성 `g:대화:요청:경로`,
 * 답 안 그림 `i:대화:요청:src`, 결과 파일 `r:대화:요청:경로`, 브라우저 캡처 `b:대화:요청:캡처번호`)이다. 같은 대상은 한 번만 나온다.
 * canSave 는 기록만으로 본 "saveFileCopy(target) 이 통과할 것 같은가"다(작업 폴더 밖 파일도 사본 저장을 받아 참이다, B-101. 열 수 없는 주소·다른 기기 대화의 작업 폴더 파일만 거짓. 디스크 확인은 하지 않아 민감 위치·링크 등으로 실제 저장이 거절할 수 있다).
 * 브라우저 캡처(kind 'browser-evidence')는 FileTarget 으로 열 수 없어 target 대신 capture 를 싣고 'browser-capture' 요청으로 읽는다(canSave 는 늘 거짓). 둘 중 하나만 있다.
 */
export type SessionImageKind = 'attachment' | 'steer-attachment' | 'reply-image' | 'generated' | 'result-file' | 'browser-evidence';
export interface SessionImageItem { key: string; kind: SessionImageKind; turnId: string; label: string; target?: FileTarget; capture?: { turnId: string; captureId: string }; canSave: boolean;
  /** 같은 그림을 가리키는 다른 열쇠. AI 가 열어 본 그림이 그 요청의 결과 파일이면 결과 파일 항목 하나로 넣고 진행 기록 미리보기의 열쇠를 여기 둔다. 크게 보기가 누른 그림 자리를 찾는다. */
  aliases?: string[] }
import type { WorkspaceFile, FolderEntry, FolderListing, WorkspaceListing, WorkspaceScope, SessionFileSource } from './workspace-files-types';
export type { WorkspaceFile, FolderEntry, FolderListing, WorkspaceListing, WorkspaceScope } from './workspace-files-types';
/** 파일함 전체 파일 보기의 한 줄. 첨부는 올린 파일, 결과 파일은 산출물이다. */
export interface SessionFileItem {
  sessionId: string;
  sessionTitle: string;
  turnId?: string;
  kind: 'attachment' | 'result';
  attachmentId?: string;
  name: string;
  path: string;
  mode?: Attachment['mode'];
  resultSource?: ResultFile['source'];
  size: number | null;
  createdAt: string;
  available: boolean;
  note?: string;
  /** note 의 키 + 인자(다국어-6). 없으면 note 문장을 그대로 보인다. */
  noteI18n?: WireText;
  prompt?: string;
  /** 기록 없는 사본. 대화 기록에도 지운 대화 사본 기록에도 없다(docs/notes/unused-cleanup-2026-09-26.md 1번). */
  loose?: boolean;
  /**
   * 새로 시작 확인창이 묶음을 나누는 자리(docs/notes/restart-continue-2026-10-02.md D5). temporary는 운영체제 임시 폴더 안(실제 경로로 폴더 경계 판정),
   * inside면 folder는 대화 작업 폴더 기준 상대 폴더(바로 아래면 ''), 아니면 폴더 전체 경로다. 대화 파일 목록에만 채운다.
   */
  place?: { temporary: boolean; inside: boolean; folder: string; /** 파일의 실제 경로(링크를 푼 것). 세 묶음 중복 제거 기준(감사 A5). */ realPath: string };
  /** 기록 하나의 안정 열쇠. 같은 기록을 다시 받아도 같고, 같은 경로라도 다른 요청·다른 첨부면 다르다(docs/notes/files-and-work-materials-2026-10-10.md 묶음 1 설계 1번). */
  key?: string;
  /** 연결 경로. steer=진행 중 요청에 붙인 추가 지시 첨부, queue=대기 요청 첨부, pending=보낼 첨부, generated=생성 그림. 요청 첨부·결과 파일이면 없다. */
  via?: (typeof import('./schedule-lists').SESSION_FILE_VIAS)[number];
  steerId?: string;
  queueId?: string;
  /** 결과 파일 중 그 요청의 변경 요약에 같은 경로가 새 파일/수정으로 있을 때만. */
  change?: 'added' | 'modified';
  /** 디스크에서 확인한 파일 수정 시각(ISO). 확인하지 못하면 없다. */
  modifiedAt?: string;
  /** 요청에 속한 항목의 요청 번호(그 대화 turns 배열 위치+1). 대기 요청·보낼 첨부에는 없다(같은 노트 묶음 2 설계 6번). */
  turnIndex?: number;
  /** 그 요청의 지금 상태. 요청에 속한 항목에만. */
  turnStatus?: TurnStatus;
  /** 그 요청을 만든 시각(ISO). 항목 createdAt 이 완료 시각일 수 있어 따로 둔다. */
  turnAt?: string;
  /** 열 수 없는 까닭의 갈래. missing=경로에 파일 없음, denied=접근 권한 없음(파일은 있으나 읽기 권한 없음 포함). 민감 위치·작업 폴더 밖 판정은 note 만 쓴다. */
  problem?: 'missing' | 'denied';
}
/**
 * 파일함 세션 보기의 거름. 백엔드가 최근 1000개로 자르기 전에 걸러 오래된 파일도 찾힌다(docs/notes/file-scope-2026-09-24.md).
 * hideBroken이면 지금 열 수 없는 파일(없음·작업 루트 밖)을 뺀다(docs/notes/file-broken-2026-09-25.md). 기록은 바꾸지 않는 보기 설정이다.
 */
export interface SessionFileFilter {
  query: string; source: SessionFileSource; kind: string; hideBroken?: boolean;
  /** 어느 대화도 쓰지 않는 사본만 본다(docs/notes/unused-cleanup-2026-09-26.md 1번). 범위는 쓰지 않는다. */
  unused?: boolean;
  /**
   * 새로 시작 확인창용 조회(docs/notes/restart-continue-2026-10-02.md 감사 A4). 대화 하나의 범위에서만 쓰고, 이 대화 기록에 있는 파일의
   * 존재와 실제 경로를 작업 루트와 관계없이 확인한다(내용은 읽지 않는다). 일반 파일함의 범위는 넓히지 않는다.
   */
  forRestart?: boolean;
  /** 화면이 지난 응답에서 받은 stamp. 지금 stamp 와 같으면 디스크 확인 없이 unchanged 로 답한다(docs/notes/files-and-work-materials-2026-10-10.md 묶음 1 설계 3번). */
  since?: string;
}
/**
 * total은 거른 뒤 전체 개수, items는 그중 최근 1000개다. broken은 거른 것 가운데 열 수 없는 수이고, hideBroken이면 total·items에서 빠져 있다.
 * deleted는 고정한 범위의 대화를 지워 빈 목록을 돌려줬다는 뜻이다(docs/notes/session-delete-2026-09-25.md 7번).
 */
export interface SessionFileList { items: SessionFileItem[]; total: number; broken: number; deleted?: boolean;
  /** 미사용 보기에서 거른 전체의 크기 합(바이트). 파일함 머리 칩과 배너가 쓴다(docs/notes/unused-cleanup-2026-09-26.md 2번). */
  size?: number;
  /** 거르기 전 기록 열쇠들의 지문. 미사용 보기에는 없다. */
  stamp?: string;
  /** 거르기 전 기록 수. */
  listed?: number;
  /** since 가 지금 stamp 와 같아 디스크 확인 없이 빈 items 로 답했다는 뜻. */
  unchanged?: true;
  /** 실행부가 이 목록을 확인한 시각(ISO). 화면이 거른 결과 밖 추가의 기준 시각으로 쓴다(docs/notes/files-and-work-materials-2026-10-10.md 실측 결함 E). */
  at?: string;
  /** 대화 하나를 물었을 때 그 대화의 가장 최근 요청(요청이 없으면 없다). prompt 는 300자로 자른다. unchanged 응답에도 들어 있다. */
  lastTurn?: { id: string; index: number; status: TurnStatus; createdAt: string; prompt: string } }
/** 미사용 사본 지우기 대상. 이름은 기록 없는 사본의 확인과 실패 알림에 쓴다(docs/notes/unused-cleanup-2026-09-26.md 3번). */
export interface UnusedFileTarget { sessionId: string; attachmentId: string; name: string }
/** 미사용 사본 지우기 결과. 지우지 못한 것은 이름과 까닭이다. */
export interface UnusedCleanup { removed: number; failed: Array<{ name: string; reason: string; reasonI18n?: WireText }> }
/**
 * 복원 뒤 데이터 폴더에 남은 옛 기록 폴더(docs/notes/restore-leftovers-2026-09-26.md 1번). old는 바꾸기 전 기록,
 * staging은 되돌린 복원 시도가 남긴 새 항목이다. contains는 든 것(대화 기록·첨부·결과 보관)이다.
 */
export interface RestoreLeftover { name: string; kind: 'old' | 'staging'; createdAt: string; bytes: number; contains: string[]; containsI18n?: WireText[] }
/** 파일함 범위 메뉴의 대화 한 줄. lastAt은 그 대화에서 가장 최근 파일의 시각이다. */
export interface SessionFileCount { sessionId: string; title: string; count: number; lastAt: string }
export type { FileExport };

/** 진단 자료에 담을 항목(docs/notes/diagnostics-2026-09-24.md 3·5번). on은 처음 켜 두는 것이다. 이름은 화면 사전과 백엔드 진단 문서(src/backend/diagnostics.ts)가 갖는다. */
export type DiagnosticsSection = 'app' | 'engines' | 'storage' | 'failures' | 'errors' | 'settings' | 'workspace' | 'log';
export const DIAGNOSTICS_SECTIONS: ReadonlyArray<{ id: DiagnosticsSection; on: boolean }> = [
  { id: 'app', on: true },
  { id: 'engines', on: true },
  { id: 'storage', on: true },
  { id: 'failures', on: true },
  { id: 'errors', on: true },
  { id: 'settings', on: true },
  { id: 'workspace', on: false },
  // 로그 파일의 끝부분(docs/notes/app-log-2026-09-24.md 8번). 길어서 맨 뒤에 둔다.
  { id: 'log', on: true },
];

/** 실행부가 만든 진단 자료(같은 문서 4번). 저장은 id로 이 글을 그대로 쓴다. 가린 곳은 종류별 개수다. */
export interface Diagnostics {
  id: string;
  text: string;
  sections: DiagnosticsSection[];
  redactions: { secrets: number; emails: number; paths: number };
}

export type { Activity } from './session-run';

import type { FileChangeSummary, FailureKind, FailureCause, AgentReport, RestartAsk, GeneratedImage, AsyncQuestion, PlanStatus, TurnPlan } from './turn-parts';
export type { FileChangeSummary, FailureKind, FailureCause, AgentReport, RestartAsk, GeneratedImage, AsyncQuestion, PlanStatus, TurnPlan } from './turn-parts';

import type { PendingRecovery, RecoveryEnd, RecoverySettings, TurnRecovery } from './recovery-types';
export type { RecoveryKind, RecoveryTarget, RecoveryStep, TurnRecovery, RecoveryEnd, PendingRecovery, RecoverySettings } from './recovery-types';

export type { SentFrom, SentFromRoot } from './core-types';

import type { AppActionRecord } from './app-action-wire';
export type { AppActionRecord, AppActionDisplay, AppActionDisplayEffect } from './app-action-wire';

export interface Turn extends TurnCancelFields {
  id: string;
  attemptId?: string;
  engineTurnId?: string;
  approvals?: Approval[];
  /** 이 요청을 보낸 곳(docs/notes/app-actions-2026-10-13.md R·D2-1). 없으면 출처 모름(옛 기록). 로컬로 추정하지 않는다. */
  sentFrom?: SentFrom;
  /** 이 요청에서 AI 가 앱 도구로 한 일의 활동 줄(같은 문서 D3). 최대 50개. */
  appActions?: AppActionRecord[];
  /** 이 요청에서 Codex 가 일을 계속하며 던진 비동기 질문(최대 20). 답을 기다리지 않는다. */
  asyncQuestions?: AsyncQuestion[];
  /** 이 요청에서 에이전트가 미리보기 브라우저로 한 일(docs/notes/preview-browser-2026-10-02.md D5 근거). 앱이 확인했다는 뜻이 아니다. */
  browser?: TurnBrowser;
  attachments?: Attachment[];
  resultFiles?: ResultFile[];
  /** 이 요청에서 Codex 가 만든 그림(최대 20, 같은 문서 D2). */
  generated?: GeneratedImage[];
  execution?: TurnExecution;
  /** 이 요청에서 AI가 남긴 새로 시작 권함(docs/notes/restart-continue-2026-10-02.md D1). */
  restartAsk?: RestartAsk;
  /** AI가 남긴 다음 지시 항목으로 만든 요청이면 그 새로 시작 기록 번호(같은 문서 D4). */
  continuation?: string;
  /** 이 요청은 끝나도 부모 대화에 결과를 보내지 않는다. 예약 회차가 notifyParent 끔으로 보낸 요청과 앱이 이어 만든 후속 요청. 사람 추가 지시가 받아들여지면 지운다(docs/notes/schedule-child-chat-2026-10-25.md L3·M1·M2). */
  noParentReport?: true;
  /** 이 요청의 파일 체크포인트 칸(docs/notes/rewind-branch-2026-10-02.md D1). 실행 모드 주 요청에만 있다. */
  checkpoint?: TurnCheckpoint;
  /** 이 요청에 적용한 답 언어와 정한 까닭(docs/notes/answer-language-2026-10-02.md L1). 요청 앞 문맥 줄과 지난 요청의 정보가 이 값을 쓴다. 이 기능 전 기록은 없다. */
  answerLanguage?: AppliedAnswerLanguage;
  /**
   * 갈래가 앞 요청을 복사한 읽기 전용 요청이면 원래 대화·요청(docs/notes/rewind-branch-2026-10-02.md D3 [논쟁 11]). 복사 요청은 사용량 합계,
   * 계획 승인, 결과 불명 복구, 위임 전달, 다시 요청·새로 시작, 자동 작업트리 첫 요청 판정, 되돌리기 동작에서 뺀다.
   */
  copiedFrom?: { sessionId: string; turnId: string };
  /** 터미널에서 가져온 읽기 전용 요청. 복사 요청과 같이 읽기 전용이지만 자동 인계에서는 늘 뺀다(docs/notes/engine-import-2026-10-19.md). */
  imported?: ImportedMark;
  /** 이 요청이 만든 계획. 계획 모드 요청이 정상 완료돼 저장에 성공했을 때만 있다(같은 문서 D7). */
  plan?: TurnPlan;
  /** 승인한 계획이 있는 계획 요청 번호. 계획대로 실행하는 요청에만 있다(같은 문서 D8). */
  planOf?: string;
  /** 고쳐 달라는 계획을 낸 계획 요청 번호. 계획을 고치는 요청에만 있다. */
  reviseOf?: string;
  /** 엔진이 알린 실제 모델(요청 모델과 따로 둔다, 같은 문서 7번). */
  engineModels?: string[];
  /**
   * 엔진이 알린 정가 환산 금액 중 이 요청 몫(달러, docs/notes/usage-2026-09-24.md 2번). 청구서 금액이 아니라 사용량 줄의 설명과
   * 대화 합계에만 "정가 환산 추정"으로 보인다. 모르면 없다(0으로 채우지 않는다).
   */
  costUsd?: number;
  /** 다른 엔진의 기록을 이어받으며 이번 요청 앞에 붙여 보낸 인계 글(docs/notes/engine-switch-2026-09-24.md 3·8번). */
  handoff?: Handoff;
  /** 작업 폴더를 옮긴 뒤 첫 요청이면 옮긴 경로. 요청 앞에 알림 글을 붙인다(docs/notes/move-folder-2026-09-26.md 6번). */
  relocated?: { from: string; to: string };
  /**
   * 이 요청 앞에 실어 보낸 위임 결과(docs/notes/delegation-2026-09-24.md 6번). 사용자 글이 없으면 결과 전달만 하는 요청이다.
   * 실은 글 전문은 화면 상태에 싣지 않고 볼 때 따로 읽는다. childIds 는 delegationIds 가운데 하위 대화 결과 번호(docs/notes/sub-sessions-2026-10-19.md).
   */
  delivery?: { delegationIds: string[]; chars: number; text?: string; childIds?: string[] };
  /** 이 요청에 적용된 규칙: 엔진이 알린 규칙 파일과 앱이 덧붙인 지시(docs/notes/rules-2026-09-24.md 1번). */
  rules?: AppliedRules;
  /** 요청이 끝난 뒤 마지막 모델 호출이 받은 문맥 크기와 모델의 문맥 창. 엔진이 알린 값만 둔다(docs/notes/auto-compaction-2026-09-24.md 1번). */
  context?: { tokens: number; max: number };
  /** 이 요청에 넣은 자료와 답이 인용한 근거 번호(docs/notes/documents-2026-10-03.md D4). 자료를 넣지 않았고 인용도 없으면 없다. 임시 대화는 메모리에만 있다. */
  evidence?: TurnEvidence;
  /** 이 요청이 참조한 다른 대화의 고정 기록(docs/notes/command-registry-2026-10-04.md 1b). 근거(evidence) 밖의 선택 칸이다. */
  chatRefs?: ChatRefRecord[];
  prompt: string;
  /** 대기 항목 여럿을 합쳐 시작한 요청의 원래 항목들(맨 앞 포함). 같은 번호의 재시도를 이 요청으로 알아본다. */
  merged?: { id: string; text: string; createdAt: string; attachmentIds?: string[] }[];
  steers?: Steer[];
  activity?: Activity[];
  /** Codex 내장 하위 에이전트 기록. 없으면 없었거나 이 기능 이전의 기록이다. */
  nativeAgents?: NativeAgent[];
  /** 진행 기록 한도를 넘어 남기지 못한 동작 수 */
  activityDropped?: number;
  status: TurnStatus;
  reply: string;
  progress: string;
  /** progress 의 키 + 인자(실행부 고정 진행 글일 때만, 다국어-8b). 없으면 progress 문장. */
  progressI18n?: WireText;
  error?: string;
  /** error 의 키 + 인자(실행부·엔진 연결이 키를 붙인 글일 때만, 묶음 5 K1). 없으면 error 문장. */
  errorI18n?: WireText;
  /** 실패 원인 분류(엔진이 알렸을 때) */
  failure?: FailureKind;
  /** retry-later 가 과부하였는지(엔진이 알렸을 때, docs/notes/failover-impl-2026-10-05.md R4) */
  failureCause?: FailureCause;
  /** 앱이 만든 이어 가기 요청이면 그 사슬 기록(같은 문서 R7). 말풍선은 글 대신 한 줄 표시를 보인다. */
  recovery?: TurnRecovery;
  /** 앱이 작업 카드 남은 일로 자동으로 이어 간 요청. of=사슬 첫 사람 요청, round=회차. 말풍선은 글 대신 한 줄 표시를 보인다. */
  autoContinue?: { of: string; round: number };
  /** 설정이 이어 가기를 원했지만 하지 않은 까닭(같은 문서 R3). */
  recoveryEnd?: RecoveryEnd;
  /** 이 요청이 붙은 요청 예산(다른 대화가 보낸 지시면 보낸 쪽 예산, docs/notes/request-budget-2026-10-24.md D6). */
  budget?: BudgetRef;
  /** 이번 요청이 바꾼 파일(엔진의 턴 전체 변경 요약, 최대 100개)과 한도를 넘어 빠진 수 */
  changes?: FileChangeSummary[];
  changesDropped?: number;
  /** 답 끝의 후속 제안(최대 3개)과 작업 결과 보고. 본문에서는 뗀다. */
  suggestions?: string[];
  report?: AgentReport;
  /** 엔진에 실행을 요청했는지. 접수 때 'pending', 요청 직전 'sent'. 없으면 옛 기록이다(docs/notes/run-coordination-2026-09-24.md 2·3번). */
  dispatch?: 'pending' | 'sent';
  dispatchedAt?: string;
  /** 중단 이유. 'app-exit'는 엔진에 보내기 전에 앱이 끝나 보내지 못한 요청이다(같은 문서 3·4번). */
  interruption?: 'app-exit';
  /** 결과 불명을 사용자가 확인한 시각. 확인하면 대화 잠금이 풀리고 상태는 결과 불명 그대로다(같은 문서 5번). */
  unknownResolvedAt?: string;
  persistenceError?: string;
  usage: Usage | null;
  createdAt: string;
  completedAt?: string;
  /** 이 요청이 엔진에 실은 공통 기억(docs/notes/common-memory-2026-10-03.md M2). 옛 요청은 칸이 없다. */
  memory?: TurnMemoryDelivery;
  /** 답 끝 기억 블록에서 뗀 후보(M3). 결과의 일부라 바꾸지 않고, 결정은 따로 저장한다. */
  memoryCandidates?: MemoryCandidate[];
  /** 자동 저장이 켜진 채 후보가 있는 결과를 확정할 때 결과와 같은 저장에 싣는 표시. 앱이 그 사이 꺼져도 시작 때 한 번 이어서 처리한다. */
  memoryAuto?: true;
}

/** 엔진이 알린 규칙 파일 하나(docs/notes/rules-2026-09-24.md 1번). label은 앱이 모르는 종류일 때 엔진이 쓴 이름이다. */
export interface RuleFile {
  path: string;
  kind: 'user' | 'project' | 'local' | 'managed' | 'memory' | 'other';
  label?: string;
  /** 엔진이 센 토큰(Claude만) */
  tokens?: number;
}

/**
 * 요청에 적용된 규칙. files가 없으면 엔진이 알리지 않았다(모름). 앱이 덧붙인 지시는 요청을 접수할 때 판과 글자 수를 적는다.
 */
export interface AppliedRules {
  files?: RuleFile[];
  /** narration: 엔진이 진행 표지 약속이 든 앱 지시를 실제로 받은 요청(docs/notes/narration-fold-2026-10-02.md 2번). 없으면 약속 없이 그린다. */
  app: { version: string; chars: number; base?: string; omitted?: ('memory' | 'web')[]; narration?: true };
  /** Claude가 요청 시작 때 알린 자동 기억 폴더(docs/notes/memory-edit-2026-09-26.md 1번) */
  memoryDir?: string;
}

export { RULE_SCOPES } from './rules-types';
export type { RuleFileView, RuleFileSave, RuleScope, CommonRulesTarget, CommonRulesScopeView, CommonRulesResult, CommonRulesConsent, CommonRulesView, MemoryFileItem, MemoryFolderView, MemoryAction, MemoryResult, CommonRulesAction } from './rules-types';

export type { EngineThread } from './engine-thread';
export type { Handoff, Compaction, RestartDocument } from './handoff-record';
export type { ImportedFolderRow, ImportedFoldersInspection, ImportedFolderOutcome, ImportedFoldersMapResult, ChatFolderProjectCandidate, ChatFolderProjectsInspection } from './folder-results';

/**
 * 대화의 작업 폴더를 옮긴 기록(docs/notes/move-folder-2026-09-26.md 5번). afterTurnId는 옮길 때의 마지막 요청이고, deliveredTurnId는
 * 옮긴 사실을 엔진에 알린 요청이다.
 */
export interface Relocation {
  id: string;
  from: string;
  to: string;
  at: string;
  afterTurnId?: string;
  deliveredTurnId?: string;
}

import type { AutoCompactFields, SetAutoCompactBody } from './context-limits';
export { AUTO_COMPACT_LEGACY, AUTO_COMPACT_TOKENS, CONTEXT_CAP_TOKENS, type AutoCompactFields, type AutoCompactTokens, type SetAutoCompactBody } from './context-limits';

export type { EngineId, AcpEngineId } from './core-types';
export type {
  ScheduleRepeat, ScheduleGithubEvent, ScheduleEventInfo, ScheduleGithubError, ScheduleGithubState,
  ScheduleSkipReason, SchedulePrecheckFail, SchedulePrecheck, ScheduleRun, ScheduleInput, Schedule, ScheduleView,
  ScheduleStop, ScheduleDoneReason, ScheduleListResult, ScheduleFinishResult,
} from './schedules';
export const ENGINE_IDS: readonly EngineId[] = ['codex', 'claude', 'agy', 'grok', 'opencode', 'kiro', 'qwen', 'gemini', 'copilot', 'cursor'];
export const ACP_ENGINE_IDS: readonly AcpEngineId[] = ['opencode', 'kiro', 'qwen', 'gemini', 'copilot', 'cursor'];
/** 더는 실행하지 않지만 기록·결과 복구·사용량은 그대로 읽는 은퇴 엔진(docs/notes/agy-engine-2026-10-05.md R5). 지금은 없다(Gemini CLI 는 11-06 에 실행 엔진으로 돌아왔다). 'none' 은 어떤 엔진 id 도 아닌 자리표시다. */
export const RETIRED_ENGINE_IDS: readonly RetiredEngineId[] = [];
export type RetiredEngineId = 'none';
/** 은퇴 엔진의 제품 이름. 지원 끝 표시는 화면(src/renderer/engine-label.ts)과 백엔드 글(src/backend/engine-names.ts)이 붙인다. */
export const RETIRED_ENGINE_LABELS: Record<RetiredEngineId, string> = { none: '' };
/** 은퇴 엔진 라벨도 들어 있어 은퇴 대화를 engineIdOf 로 읽은 값으로 찾아도 제품 이름이 나온다. */
export const ENGINE_LABELS: Record<EngineId, string> = { codex: 'Codex', claude: 'Claude', grok: 'Grok', agy: 'Antigravity', opencode: 'OpenCode', kiro: 'Kiro', qwen: 'Qwen Code', gemini: 'Gemini CLI', copilot: 'GitHub Copilot', cursor: 'Cursor', ...RETIRED_ENGINE_LABELS } as Record<EngineId, string>;
/** 실행 정보 없이 운영 Lair에서 가져온 요청의 출처(docs/notes/lair-import-2026-09-24.md). 인계 출처 목록에 엔진 대신 들어간다. */
export const LAIR_ORIGIN = 'lair';
/** 대화의 웹 검색 정책. off 면 엔진의 웹 도구를 끄고, sites 는 호스트 이름(소문자, 하위 도메인 포함) 범위다. */
export interface SessionWebPolicy { off?: true; sites?: string[] }

export interface SessionChatFolder { root: string; path?: string; made?: true; fallback?: string }
/**
 * 설정 제안 카드 한 줄의 상태(docs/notes/settings-assistant-2026-10-05.md 설계 5절).
 * pending 대기, applied 적용하고 다시 읽은 값이 바꿀 값과 같음, failed 적용 요청이 거절됐거나 다시 읽은 값이 다름,
 * declined 사용자가 접음, reverted 되돌림, stale 제안 뒤 값이 이미 바뀌어 적용하지 않음.
 */
export type SettingsProposalItemState = 'pending' | 'applied' | 'failed' | 'declined' | 'reverted' | 'stale';
export interface SettingsProposalItem {
  id: SettingId;
  /** 대화·프로젝트 범위 항목의 대상. 제안 때 정하고 바꾸지 않는다. */
  target?: { sessionId?: string; projectId?: string };
  /** 제안 때 읽은 지금 값과 바꿀 값. 설정의 그 칸을 여는 항목('open')은 둘 다 없다. */
  from?: SettingValue;
  to?: SettingValue;
  risk: SettingRisk;
  /** 모델이 적은 이유(앞뒤 공백을 지우고 400자까지) */
  reason: string;
  state: SettingsProposalItemState;
  /** 적용 직전에 읽은 값(되돌릴 값)과 적용 뒤 다시 읽은 실제 값 */
  before?: SettingValue;
  actual?: SettingValue;
  /**
   * 묶인 기록의 비교 열쇠(논쟁 F5·F6). 위임 방식 항목은 그 범위 정책 전체, 새 대화 기본값 항목은 엔진·엔진별 모델·강도·계정(승인 방식 제외)을
   * 정규화한 JSON 이다. fromKey 는 제안 때, actualKey 는 적용 직후 값이다. 표시 값(from·actual)이 같아도 열쇠가 다르면 바뀐 것으로 본다.
   */
  fromKey?: string;
  actualKey?: string;
  /** 묶인 기록의 적용 직전 원본(위임 정책 또는 새 대화 기본값 기록, 없었으면 null). 백엔드만 읽고 되돌리기가 그대로 되살린다. */
  beforeRecord?: unknown;
  /** 적용을 시작했다고 먼저 저장한 시각(논쟁 F4). 결과를 적기 전에 앱이 꺼지면 다음 기동 때 실제 값을 다시 읽어 결과를 정한다. */
  applyingAt?: string;
  /** 적용은 되지만 실제로 쓰이려면 더 정할 것(논쟁 F9). recovery-order 는 넘길 순서가 비어 있음이다. */
  needs?: 'recovery-order';
  /**
   * 실패·건너뜀 이유 갈래. 화면은 사전(settingsAi.problem.*)의 합쇼체 글로 보인다. changed 제안 뒤 값이 바뀜, rejected 설정 요청이 거절됨,
   * mismatch 다시 읽은 값이 바꿀 값과 다름, target 대상 대화·프로젝트가 없어짐, unavailable 지금은 고를 수 없는 값(엔진 준비 안 됨 등).
   */
  problem?: 'changed' | 'rejected' | 'mismatch' | 'target' | 'unavailable';
  /** 거절한 설정 요청의 원래 오류 글(접은 자세히에만 보인다, 400자까지) */
  detail?: string;
  /** 마지막으로 상태가 바뀐 시각 */
  at?: string;
}
/** 모델이 propose_settings 로 낸 제안 하나. 카드 안 항목은 items 의 번호(0부터)로 가리킨다. */
export interface SettingsProposal {
  id: string;
  turnId: string;
  createdAt: string;
  items: SettingsProposalItem[];
  /** 모델이 붙인 한 줄 요약(200자까지) */
  note?: string;
  /** 이 제안이 묶인 'AI와 설정하기' 주제 번호(제안을 낸 턴부터 거슬러 올라 가장 가까운 주제 표식) */
  topic?: string;
  revertedAt?: string;
}

export interface Session extends SessionCancelFields {
  id: string;
  title: string;
  /** 붙은 태그 번호(docs/notes/b432-conversation-tags-2026-10-29.md). 없거나 지운 태그를 가리키면 태그 없음. */
  tagId?: string;
  /** 지시문 언어(다국어-7 G1, docs/notes/i18n7-engine-text-2026-10-05.md). 대화를 만들 때 이 컴퓨터 화면 언어로 정해 바꾸지 않는다. 없으면 ko. */
  instructionLanguage?: 'ko' | 'en';
  /** 임시 대화. 백엔드 메모리에만 있고 디스크에 저장하지 않는다(docs/notes/temporary-chat-2026-10-03.md T4). */
  temporary?: true;
  customTitle?: boolean;
  pinned?: boolean;
  folderId?: string;
  /**
   * 저장된 프로젝트 소속(등록 프로젝트 번호). 정리 폴더·작업 경로와 서로 독립이고 경로로 다시 짐작하지 않는다. 형식 15 이후 칸이 없으면 소속 없음이다
   * (docs/notes/sidebar-scope-2026-10-02.md S3·논쟁 2).
   */
  projectId?: string;
  /** 이 대화의 위임 정책 덮어쓰기(docs/notes/role-policy-2026-10-05.md 설계 2절). 'set-role-policy' 로만 바꾼다. 없으면 프로젝트·전역을 따른다. */
  rolePolicy?: RolePolicy;
  /**
   * 저장된 정책을 읽은 상태(없으면 ok). retired=은퇴 엔진 대상만 빠짐, unknown=모르는 칸·값을 버림, invalid=못 읽어 rolePolicy 없음.
   * 원문(rolePolicyRaw)은 백엔드 메모리에만 있고 저장 때 rolePolicy 자리에 되쓴다. 화면·원격에는 싣지 않는다.
   */
  rolePolicyState?: 'retired' | 'unknown' | 'invalid';
  /** 이 대화 정책 층의 판(불투명 문자열). 화면용이라 저장하지 않고, 대화 상태 뷰에만 채운다. */
  rolePolicyRevision?: string;
  /** 소속 없는 대화가 위임 정책을 물려받는 등록 프로젝트(작업 폴더를 품은 가장 깊은 것, docs/notes/project-policy-path-2026-10-13.md). 화면용, 대화 상태 뷰에만 채운다. */
  policyProjectId?: string;
  /** 이 대화에서는 프로젝트 자료를 넣지 않는다(docs/notes/documents-2026-10-03.md D3·D8). 없으면 넣는다. 'set-session-sources' 로만 바꾼다. */
  sourcesOff?: true;
  /** 이 대화의 웹 검색 정책(docs/notes/web-sources-2026-10-03.md W1). 없으면 켬·범위 없음. 'set-session-web' 로만 바꾼다. */
  web?: SessionWebPolicy;
  /** 이 대화에서는 공통 기억을 싣지도 후보를 만들지도 않는다(docs/notes/common-memory-2026-10-03.md M5). 'set-session-memory' 로만 바꾼다. */
  memoryOff?: true;
  /** 이 대화에서 끈 앱 MCP 서버 번호(docs/notes/app-connect-2026-10-04.md D5). 다음 요청부터 중계에서 뺀다. 'set-session-mcp' 로만 바꾼다. 형식 17. */
  mcpOff?: string[];
  /** 이 대화에서 "계속 허용"한 명령(최대 50개). 같은 엔진·도구·명령 글자·작업 폴더의 명령 승인은 묻지 않고 허용한다. 갈래·가져오기·새 대화로는 복사하지 않는다. */
  commandAllows?: CommandAllow[];
  /**
   * 예약이 만든 대화(docs/notes/schedules-2026-10-04.md R5). 승인 방식은 일반 대화와 같은 규칙을 따른다.
   * 예약을 지워도 남는다. 형식 19. 옛 데이터의 approval 칸은 읽어도 쓰지 않는다.
   */
  scheduled?: { scheduleId: string; runId: string; name: string };
  projectPath: string;
  /**
   * 이 대화의 다음 요청을 받을 엔진. 없으면 Codex(엔진 선택 전의 기록)다. 바꾸면 다음 요청부터 적용하고, 지난 요청의 엔진은
   * 요청마다 고정한 실행 선택에 남는다(docs/notes/engine-switch-2026-09-24.md 1번).
   */
  engine?: EngineId;
  model: string;
  /** 다음 요청의 생각 강도. null·없음은 엔진 기본이다(docs/notes/effort-approval-2026-10-01.md 1.2). */
  effort?: string | null;
  /** 다음 요청의 속도 등급 id(엔진이 알린 speeds 의 id). 없으면 엔진 기본이다(docs/notes/engine-models-2026-10-28.md D4·D5). */
  speed?: string;
  /** 다음 요청의 모드. 없으면 실행이다(docs/notes/plan-mode-2026-10-02.md D1). */
  mode?: RequestMode;
  /** 옛 대화별 승인 방식. 이제 읽지 않고 데이터만 그대로 둔다. 실제 승인 방식은 설정 한 값이다(docs/notes/switch-composer-2026-10-01.md 3.1). */
  approval?: ApprovalMode;
  /** 대화에서 사람이 정한 승인 방식(docs/notes/b270-approval-owner-2026-10-22.md D3). 옛 approval 필드와 다르다(그 필드는 읽지 않음). 없으면 예약 값·앱 설정을 따른다. 형식 42. */
  approvalOverride?: ApprovalMode;
  /** 다음 요청을 받을 추가 계정 번호. 없으면 그 엔진의 기본 계정이다(docs/notes/accounts-2026-09-26.md 9번). */
  account?: string;
  /**
   * 참여자 대화(docs/notes/participant-runs-2026-10-04.md R4·M10). 프로젝트 참여자가 원격으로 만든 대화이고, 그 사람 기기만 보내고·멈추며
   * 실행은 참여자 실행 경계 안에서 그 사람 API 키로만 돈다. 실행부가 만들 때 관문이 실은 참여자 표시로 정하고 바뀌지 않는다. 동기화로 내보내지 않는다.
   */
  runAs?: { personId: string; name: string };
  /**
   * 엔진·계정별 엔진 대화. 열쇠는 threadSlot(엔진, 계정)이다. 모르는 엔진의 칸도 지우지 않고 둔다(같은 문서 2번,
   * docs/notes/accounts-2026-09-26.md 9번).
   */
  engineThreads?: Partial<Record<string, EngineThread>>;
  /** 바깥 터미널 대화를 이어 연 대화의 출처(docs/notes/engine-import-2026-10-19.md) */
  externalThread?: ExternalThread;
  createdAt: string;
  updatedAt: string;
  /** 사용자가 이 시각까지 끝난 결과를 봤다(docs/notes/attention-2026-09-23.md 2번). */
  seenAt?: string;
  turns: Turn[];
  pendingAttachments?: Attachment[];
  /** 입력창에서 뺀 영상 장면(되돌릴 수 있게 보관). 상한 EXCLUDED_SCENES_MAX, 보내면 형제 없는 묶음은 버린다(b559 D2). */
  excludedScenes?: Attachment[];
  queue?: QueuedRequest[];
  /** 문맥 압축 기록. 최근 50개까지 둔다(docs/notes/compaction-2026-09-24.md 5번). */
  compactions?: Compaction[];
  /** 작업 폴더를 옮긴 기록. 최근 20개까지 둔다(docs/notes/move-folder-2026-09-26.md 5번). */
  relocations?: Relocation[];
  /** 파일 되돌리기 결과 줄. 최근 20개까지 둔다(docs/notes/rewind-branch-2026-10-02.md D2). */
  restores?: RestoreMark[];
  /** 예약된 이어 가기(docs/notes/failover-impl-2026-10-05.md R8). 있으면 대기 실행·예약 실행이 이 대화를 시작하지 않는다. */
  recovery?: PendingRecovery;
  /** 요청 전체 예산 하나(docs/notes/request-budget-2026-10-24.md). 닫힌 예산도 다음 예산이 열릴 때까지 남는다. */
  budget?: RequestBudget;
  /** 메인 모델이 맡긴 위임. 최근 50개와 진행 중인 것을 둔다(docs/notes/delegation-2026-09-24.md 5번). */
  delegations?: Delegation[];
  /** 하위 대화면 있다: 부모와 마지막 보고 요청(docs/notes/sub-sessions-2026-10-19.md 4.2). */
  parent?: SessionParent;
  /** 부모 쪽에 고정한 하위 결과. 배달한 것은 최근 50개, 미배달은 지우지 않는다. */
  childResults?: ChildResult[];
  /**
   * AI와 설정하기 대화(docs/notes/settings-assistant-2026-10-05.md 설계 4절). 이 대화에만 propose_settings 도구와 설정 목록을 준다.
   * fromSessionId 는 입력칸 명령으로 연 대화이고 대화·프로젝트 범위 항목의 대상이다. 'open-settings-assistant' 로만 생긴다.
   */
  settingsAssistant?: { fromSessionId?: string; doneTopics?: string[]; topics?: { turnId: string; show: boolean } };
  /** 설정 제안 카드. 최근 30개를 둔다. 상태는 'settings-proposal' 로만 바뀐다(설계 5절). */
  settingsProposals?: SettingsProposal[];
  /** 운영 Lair에서 가져온 대화의 원본 번호와 원본의 마지막 변경 시각. 같은 원본을 다시 가져오지 않는 데 쓴다(docs/notes/lair-import-2026-09-24.md). */
  importedFrom?: { source: 'lair'; id: string; updatedAt: string; importedAt: string };
  /** 엔진 터미널 대화면 있다(docs/notes/terminal-slice2-2026-09-27.md 1번). 엔진은 대화의 engine이다. */
  terminal?: EngineTerminal;
  /** 이 대화가 쓰는 작업트리 번호. 있으면 projectPath 가 그 작업트리 디렉터리다(docs/notes/worktree-2026-10-02.md D2). */
  worktreeId?: string;
  /**
   * 첫 실행 요청을 보낼 때 작업트리를 만들 예약(같은 문서 D2). base 는 기준 브랜치 이름이고 없으면 원본의 지금 브랜치다.
   * auto 는 같은 저장소에 쓰는 대화가 겹쳐 앱이 건 것이다(D1). 만들고 나면 지운다.
   */
  isolation?: SessionIsolation;
  /** 프로젝트 없이 연 새 대화의 대화 폴더 예약(docs/notes/no-blocking-2026-10-03.md N5). root=만들 때의 앱 작업 루트, path=만들 경로(만들기 전 저장), made=만들어 작업 폴더로 확정, fallback=못 만들어 루트로 시작한 까닭. */
  chatFolder?: SessionChatFolder;
  /** 갈래로 만든 대화면 묶음 위치(docs/notes/rewind-branch-2026-10-02.md D3). 원래 대화에는 없다. */
  branch?: SessionBranch;
  /** 기록 동기화(docs/notes/encrypted-sync-2026-10-04.md S5). 이어 가는 기기가 이 기기가 아니면 실행 요청을 거절한다. 동기화를 켜지 않았으면 없다. */
  sync?: SessionSync;
}

/** 갈래 종류: 되돌리기·고쳐 보내기·다른 답·다른 모델(같은 문서 D3) */
export type BranchKind = 'rewind' | 'edit' | 'regenerate' | 'compare';
export const BRANCH_KINDS: readonly BranchKind[] = ['rewind', 'edit', 'regenerate', 'compare'];
export type { BranchInput };
/**
 * 갈래 만들기가 "다시 싣지 못하는 입력이 있어 멈췄다"는 오류 글의 머리말(감사 K10, docs/notes/rewind-branch-2026-10-02.md). 뒤에 줄마다 못 싣는 입력 하나가 온다.
 * send.allowMissing 을 싣고 다시 보내면 빼고 보낸다. send.check 는 아무것도 만들지 않고 이 검사까지만 한다(되돌리기 전에 미리 알려 주는 데 쓴다).
 */
export const BRANCH_MISSING_PREFIX = 'LAIR_BRANCH_MISSING:';
/**
 * 갈래의 연결(같은 문서 D3). rootId 는 묶음의 원래 대화(갈래의 갈래도 같은 값), fromSessionId 는 바로 갈라 나온 대화, atTurnId 는 그 대화에서
 * 갈라진 요청이다. 갈라진 요청 앞의 요청들이 복사 요청으로 들어 있다. startOp 는 "대화와 파일 둘 다"일 때 파일을 되돌린 작업 번호다.
 */
export interface SessionBranch {
  rootId: string;
  fromSessionId: string;
  atTurnId: string;
  kind: BranchKind;
  startOp?: string;
  /** 만들 때 받은 요청 번호. 같은 번호로 다시 오면 같은 대화를 돌려주는 데 쓴다. */
  requestId?: string;
  /** 고쳐 보내기·다른 답 받기가 실어 보내는 첫 요청(docs/notes/rewind-branch-2026-10-02.md D4). 없으면 첫 요청을 싣지 않은 갈래다. */
  first?: BranchFirst;
  /** 그때 입력 중 기록에 없어 다시 싣지 못한 것의 이름들(같은 문서 D4 [논쟁 10]). 화면이 알린다. */
  missing?: string[];
  /** 다른 모델로 받기의 비교 기록(같은 문서 D5). compare 갈래만 갖는다. */
  compare?: BranchCompare;
  createdAt: string;
}

/**
 * 다른 모델로 받기의 비교 기록(같은 문서 D5). where 는 돌린 곳: worktree 는 원래 요청 앞 상태를 만든 0-3 작업트리(실행 요청), folder 는 같은 폴더(조사·계획 요청, 비교할 파일 변경 없음).
 * origin 은 원래 요청 앞 스냅숏, head 는 그때의 HEAD 커밋, baseline 은 작업트리에서 시작 상태를 다시 뜬 비교 기준 스냅숏이다(작업트리를 만든 뒤에 채운다).
 */
export interface BranchCompare {
  where: 'worktree' | 'folder';
  origin?: string;
  head?: string;
  baseline?: string;
  /** 작업트리를 만든 뒤 기록하는 작업트리 번호(정리 권유에 쓴다) */
  worktreeId?: string;
}

/**
 * 갈래 첫 요청의 재현 입력(같은 문서 D4). id 는 첫 요청 번호(폴더 대기를 거쳐도 같다). planTurnId 는 계획대로 실행하는 요청이면 이 갈래에 복사된 계획 요청
 * 번호(과거 승인은 쓰지 않고 이번 보내기의 승인으로 실행한다), delivery 는 그 요청에 실렸던 위임 결과 전달 글이다. error 는 첫 요청을 보내지 못한 까닭이다
 * (이때 입력칸에 글·첨부를 담아 둔다).
 */
/** 요청이 참조한 다른 대화 하나의 고정 기록(docs/notes/command-registry-2026-10-04.md 1b). excerpt 는 엔진에 실은 발췌 전문이고 화면 상태에서는 지운다. */
export interface ChatRefRecord { id: string; title: string; readAt: string; excerpt?: string; trimmed?: true; missing?: 'not-found' | 'temporary' | 'self' | 'over-limit' | 'no-room'; chars?: number }

export interface BranchFirst {
  id: string;
  planTurnId?: string;
  delivery?: { delegationIds: string[]; chars: number; text: string };
  /** 재현 대상 요청의 적용 답 언어 코드. 앱이 만든 첫 요청이 "내가 쓴 말 따라"에서 언어를 잃지 않게 한다(docs/notes/answer-language-2026-10-02.md 감사 K4). */
  answerLanguage?: EngineLanguage;
  /** 다른 답·비교 갈래가 원래 요청의 발췌를 그대로 싣는다(docs/notes/command-registry-2026-10-04.md 논쟁 1) */
  chatRefs?: ChatRefRecord[];
  error?: string;
}

/** 대화의 작업트리 만들기 예약(docs/notes/worktree-2026-10-02.md D2) */
export interface SessionIsolation {
  kind: 'worktree' | 'copy';
  base?: string;
  auto?: boolean;
  /**
   * 다른 모델 실행 비교의 시작 상태(docs/notes/rewind-branch-2026-10-02.md D5 [논쟁 7]). oid 는 원래 요청 앞 체크포인트에 고정한 그때의 HEAD 커밋, checkpointId 는 그 앞 스냅숏,
   * rel 은 저장소 최상위 기준 상대 경로다. 있으면 작업트리를 그 커밋에서 만들고 포함 범위를 그 스냅숏과 같게 만든 뒤 준비 명령으로 간다.
   */
  from?: { oid: string; checkpointId: string; rel: string };
}

export type { WorktreeStage, WorktreeMergeMethod, WorktreeMergeStep, WorktreeMerged, WorktreeSetup, WorktreeCleanPreview, WorktreeMergeResult, WorktreeIncludePreview } from './worktree-types';
export type { TurnCheckpoint, CheckpointFile } from './session-run';

export type CheckpointStage = 'before' | 'after' | 'op';
import type { CheckpointExcludeReason } from './file-version-types';
export type { CheckpointExcludeReason } from './file-version-types';

/** 체크포인트(스냅숏) 명세와 변경 기록 한 줄. 앱 기록 표 checkpoints 에 저장한다(docs/notes/rewind-branch-2026-10-02.md D1). */
export interface CheckpointRecord {
  id: string;
  key: string;
  dir: string;
  worktreeId?: string;
  sessionId: string;
  turnId: string;
  stage: CheckpointStage;
  /** op 일 때 작업 번호 */
  opId?: string;
  ref: string;
  commit: string;
  at: string;
  /** 포함한 경로 수 */
  included: number;
  excluded: Array<{ path: string; reason: CheckpointExcludeReason }>;
  excludedCounts: Partial<Record<CheckpointExcludeReason, number>>;
  excludedTruncated?: true;
  /** 직전 스냅숏(같은 열쇠)과 그와의 변경 경로 */
  prev?: string;
  changes: Array<{ path: string; status: string }>;
  changesOverflow?: true;
  /** 변경 주인: before = 요청 밖, after = 그 요청, op = 작업 */
  owner: 'outside' | 'request' | 'op';
  mixed?: true;
  /** 결과 불명으로 끝난 요청의 뒤 */
  unconfirmed?: true;
  /** 직전 스냅숏 이후 이 범위에서 체크포인트를 못 남긴 요청이 있었음 */
  broken?: true;
  /** 이 스냅숏을 뜰 때 동시 실행 이력에 있던 다른 대화 번호들. 있으면 이 기록의 변경에 다른 대화 변경이 섞였을 수 있다. */
  peers?: string[];
  /** peers 가 50개를 넘어 더 적지 못했다. 켜져 있으면 peers 가 있는 것과 같게 다룬다. */
  peersOverflow?: true;
  /** 사슬의 직전 기록이 동시 실행 상대 요청의 앞 스냅숏이라 생긴 끊김(broken 이 아니다) */
  interleaved?: true;
  /** 사슬 밖 스냅숏(쓰기 위임의 기준, docs/notes/write-subtasks-2026-10-04.md 고침 F1). 같은 열쇠의 직전 기록·변경 목록·판 기록·되돌리기 사슬에 끼지 않는다. 번호로만 읽는다. */
  detached?: true;
}

/** 되돌리기 작업 기록(앱 기록 표 restore_ops, 대화와 독립이다) */
export interface RestoreOp {
  id: string;
  sessionId: string;
  kind: RestoreKind;
  /** rewind 면 목표 요청, undo 면 되돌릴 작업 번호 */
  targetTurnId?: string;
  undoOf?: string;
  key: string;
  dir: string;
  stage: RestoreStage;
  /** 앞 스냅숏(op 단계)과 뒤 스냅숏 번호 */
  before?: string;
  after?: string;
  /** 목표 스냅숏 번호(되돌리는 상태) */
  target?: string;
  results: RestoreFileResult[];
  requestId: string;
  createdAt: string;
  updatedAt: string;
  planHash: string;
  /** 실패·중단 까닭 */
  error?: string;
  /** 보호 중 덮어쓰기로 고른 경로 */
  overwrite?: string[];
  /** 적용(kind apply)일 때 원래 대화·요청과 적용한 갈래의 답·그 답의 뒤 체크포인트·비교 기준(sessionId 는 적용을 누른 갈래 대화, key·dir 은 원래 폴더) */
  apply?: { originSessionId: string; originTurnId: string; branchTurnId: string; branchAfter: string; baseline: string; worktreeId?: string };
  /** 작업을 시작할 때의 Git HEAD 와 저장소 최상위 기준 상대 경로(감사 K8). 되돌린 상태에서 시작하는 갈래가 자동 작업트리를 그 커밋에서 만들 때 쓴다. 없으면 모른다. */
  git?: { head?: string; rel: string };
  /** 마무리 뒤 늦게 확인된 휴지통 지움(감사 고침 확인 R5): 그때 뜬 요청 밖 스냅숏 번호와 그 안에서 이 작업의 것으로 보는 지움 경로 */
  late?: { snapshot: string; paths: string[] };
}
/** 대화 칸의 되돌리기 줄(docs/notes/rewind-branch-2026-10-02.md D2 결과 줄). afterTurnId 는 그때의 마지막 요청이다. */
export interface RestoreMark {
  id: string;
  kind: RestoreKind;
  targetTurnId?: string;
  /** 목표 요청 글의 첫 줄(화면 표시용) */
  title: string;
  afterTurnId?: string;
  stage: RestoreStage;
  wrote: number;
  trashed: number;
  skipped: number;
  /** 휴지통 결과를 알 수 없는 항목 수(감사 K4). 옛 줄에는 없다. */
  unknown?: number;
  at: string;
  /** 이 되돌리기를 취소했거나(undone), 중단된 것을 다시 했다(redone) */
  resolved?: 'undone' | 'redone';
  /** 적용 줄이면 갈래의 작업트리 번호(결과 줄이 정리를 권한다) */
  worktreeId?: string;
}

/** branch-excluded: 갈래의 답 뒤 스냅숏에서 제외(크기·확인 실패·명세 잘림 등)라 갈래가 지운 것인지 알 수 없음(감사 K2) */
export type RestoreProtectReason = 'outside' | 'other-session' | 'shared' | 'mixed' | 'unconfirmed' | 'broken' | 'changed' | 'overlap' | 'unknown' | 'later' | 'branch-excluded';
/** 계획의 경로 한 줄. protect 는 기본으로 덮지 않고, wouldDo 는 덮어쓰기를 고르면 할 동작이다. */
export interface RestorePlanItem {
  path: string;
  group: 'restore' | 'trash' | 'protect';
  wouldDo?: 'write' | 'trash';
  reason?: RestoreProtectReason;
  /** 사람이 읽을 까닭 */
  note?: string;
  /** note 의 키 + 인자(실행부 고정 글일 때만). 없으면 note 문장. */
  noteI18n?: WireText;
  /** other-session 일 때 그 대화 */
  sessionTitle?: string;
  /** sessionTitle·note 가 가리키는 대화가 임시 대화(폰이 이름 대신 "임시 대화"로 그린다) */
  temporary?: true;
  /** 적용 계획: 이 경로의 목표를 어디서 가져오나(branch 갈래의 답 내용, origin 원래 요청 앞 상태)와 갈래 쪽 변경 종류(A 추가·M 수정·D 삭제) */
  source?: 'branch' | 'origin';
  branchStatus?: string;
}
export interface RestorePlan {
  ok: boolean;
  /** ok 가 false 일 때 되돌릴 수 없는 까닭 */
  reason?: string;
  /** reason 의 키 + 인자(실행부 고정 글일 때만, 묶음 5 K4). 없으면 reason 문장. */
  reasonI18n?: WireText;
  sessionId: string;
  kind: RestoreKind;
  turnId?: string;
  opId?: string;
  /** 목표 요청 글의 첫 줄 */
  title: string;
  hash: string;
  items: RestorePlanItem[];
  /** 이 요청 뒤 같은 열쇠에서 돈 다른 대화 */
  otherSessions: Array<{ sessionId: string; title: string; temporary?: true }>;
  /** 적용 계획이면 갈래의 작업트리 번호(적용 뒤 정리 권유용) */
  worktreeId?: string;
  /** 적용 계획의 기준이 준비 명령 앞의 비교 기준이라 준비 명령의 변경이 섞였을 수 있음(감사 K14) */
  prepMixed?: true;
}
/** 'restore-run'·'restore-undo' 의 응답. pendingTrash 가 있으면 화면이 휴지통으로 옮긴 뒤 'restore-trashed' 로 알려야 끝난다. */
export interface RestoreRunResult {
  opId: string;
  kind: RestoreKind;
  stage: RestoreStage;
  results: RestoreFileResult[];
  wrote: number;
  trashed: number;
  skipped: number;
  /** 휴지통 결과를 알 수 없는 항목 수(감사 K4) */
  unknown?: number;
  pendingTrash?: Array<{ folder: string; path: string }>;
  error?: string;
}

export interface WorktreeRecord {
  id: string;
  /** worktree 는 Git 작업트리, copy 는 Git 이 아닌 폴더의 격리 복사본(docs/notes/worktree-2026-10-02.md D8). copy 는 branch·baseRef·baseOid 가 빈 글이다. */
  kind: 'worktree' | 'copy';
  /** 원본 저장소 최상위 실제 경로 */
  sourceRoot: string;
  /**
   * 시작 입력·준비에 쓰는 원본 작업 폴더의 Git 최상위(감사 고침 확인 R8). 작업트리 W1 안에서 만든 W2 는 Git 관리에 쓰는 sourceRoot(주 저장소)와 다르게 W1 이 된다.
   * .worktreeinclude 복사·승인·준비 명령은 이 폴더 기준이다. 주 저장소에서 만들었거나 옛 기록이면 없고, 그때는 sourceRoot 가 같은 뜻이다.
   */
  prepRoot?: string;
  /** 원본 프로젝트(등록 프로젝트가 원본 최상위일 때) */
  projectId?: string;
  /** 작업트리 디렉터리 실제 경로 */
  dir: string;
  /** 만들 때의 관리 위치(그때의 <작업 루트>/.lair-worktrees 실제 경로). 작업 루트를 바꿔도 정리가 이 위치 바로 아래만 지운다(docs/notes/worktree-2026-10-02.md 감사 K10). 옛 기록에는 없다. */
  managedDir?: string;
  /** 앱이 만든 브랜치 이름 */
  branch: string;
  /** 기준 브랜치 이름(원본이 detached 였으면 빈 글) */
  baseRef: string;
  baseOid: string;
  stage: WorktreeStage;
  sessionIds: string[];
  /** 만들기 요청 번호(첫 요청의 요청 번호). 같은 번호면 같은 작업이다. */
  requestId: string;
  createdAt: string;
  lastUsedAt: string;
  error?: string;
  /** error 의 앱 고정 문구 전송 키(error 와 함께 바뀌고 지워진다. Git 원문이 섞인 error 에는 없다) */
  errorI18n?: WireText;
  /** 포트 묶음의 첫 번호. 이 작업트리는 port ~ port+9 를 쓴다(환경 변수 LAIR_PORT 는 첫 번호, D3). 옛 기록에는 없다. */
  port?: number;
  /** 마지막 준비 명령 기록 */
  setup?: WorktreeSetup;
  /** 합치는 중(stage merging)의 단계 기록. 복구 필요일 때도 판정 근거로 남긴다(D4). */
  merge?: WorktreeMergeStep;
  /** 합친 기록(오래된 것부터). 마지막이 가장 최근이다(D4·D6). */
  merged?: WorktreeMerged[];
  /** 복구 필요일 때 원본 `git status --short` 앞 30줄(D4 [논쟁 1]) */
  recoveryStatus?: string;
  /** 격리 복사본(kind copy)의 명세 파일과 개수(D8). sourceRoot 는 원래 폴더 실제 경로다. */
  copy?: CopyInfo;
  /** GitHub 에서 머지된 PR 의 원격 머지 증거(오래된 것부터, 최근 20개). 로컬 합친 기록(merged)과 따로 둔다(docs/notes/github-pr-2026-10-02.md D5·논쟁 4). */
  remoteMerges?: WorktreeRemoteMerge[];
}

/** 원격 머지 증거 한 건: PR 번호·주소·머지 시각·머지된 헤드 OID(docs/notes/github-pr-2026-10-02.md D5) */
export interface WorktreeRemoteMerge { number: number; url: string; mergedAt: string; headOid: string }

/** 격리 복사본의 명세 요약(docs/notes/worktree-2026-10-02.md D8). 파일마다 명세는 앱 데이터 폴더의 manifestFile 에 있다. */
export interface CopyInfo {
  manifestFile: string;
  /** 복사한 파일 수·합계 크기, 뺀 항목 수(링크·규칙·큰 파일 등) */
  included: number;
  bytes: number;
  excluded: number;
  /** 마지막 반영 요약 */
  lastApply?: { at: string; written: number; skipped: number; failed: number; backupDir?: string };
}

/** 화면용 작업트리 보기: 기록 + 연결 대화 제목 + 앱 작업 루트 밖 여부 */
export interface WorktreeView extends WorktreeRecord {
  sessions: Array<{ id: string; title: string }>;
  /** 앱 작업 루트가 바뀌어 이 디렉터리가 새 루트 밖이다(D2 마지막 항목). 정리만 할 수 있다. */
  outOfRoot?: true;
  /** 이 작업트리 폴더의 PR 요약(열린 것 우선, 최근 갱신 하나). 사이드바 작업트리 줄 칩용(docs/notes/github-pr-2026-10-02.md 조각 3) */
  pr?: { number: number; state: GithubPrRecord['state']; isDraft: boolean; checksSummary: GithubChecksSummary; url: string };
}

/** 작업트리 하나의 지금 상태(docs/notes/worktree-2026-10-02.md D5, 'worktree-status' 요청의 응답) */
export interface WorktreeStatus {
  id: string;
  /** 작업 루트 밖이거나 폴더가 없어 Git 을 부르지 않았다. */
  outOfRoot?: true;
  missing?: true;
  head?: string;
  /** 지금 체크아웃된 브랜치. detached 면 없다. */
  branch?: string;
  /** 지금 브랜치가 앱이 만든 브랜치인가 */
  ownBranch?: boolean;
  /** 기준 대비 앞섬(작업트리에만 있는 커밋)과 뒤처짐(기준에만 있는 커밋). 기준을 못 읽으면 없다. */
  ahead?: number;
  behind?: number;
  /** 커밋 안 한 변경(추적 변경 + 추적 안 된 파일) 수 */
  dirty?: number;
  error?: string;
  /** error 의 앱 고정 문구 전송 키(Git 원문이 섞인 error 에는 없다) */
  errorI18n?: WireText;
}

/** 로컬 브랜치 한 줄(docs/notes/worktree-2026-10-02.md D7). usedBy 는 다른 작업트리가 체크아웃한 디렉터리다. */
export interface BranchInfo {
  name: string;
  current?: true;
  /** 다른 작업트리가 체크아웃한 디렉터리. 있으면 전환할 수 없다. */
  usedBy?: string;
  /** 마지막 커밋 시각(ISO)과 제목 */
  time?: string;
  subject?: string;
}
/** 기준 대비 바뀐 파일 한 줄(D4 검토 [논쟁 10]). path 는 작업트리 맨 위 기준 경로('/' 구분)다. */
export interface BaseChangeEntry { path: string; kind: (typeof import('./schedule-lists').GIT_BASE_CHANGE_KINDS)[number]; added?: number; removed?: number; binary?: boolean }
/** 병합 기준점부터 작업트리 HEAD 까지 바뀐 파일(D4). base 는 기준 이름(브랜치가 아니었으면 커밋 앞 7자). */
export interface BaseChanges {
  base: string; mergeBase: string; head: string; entries: BaseChangeEntry[]; dropped: number;
  /** 다른 모델 갈래의 비교 기준 보기일 때만(감사 K12): 비교한 뒤 스냅숏 번호, 지금 작업트리를 떴는지, 준비 명령의 변경이 섞였을 수 있는지 */
  compare?: { after: string; live: boolean; prepMixed: boolean; turnId?: string };
}
/** 기준 대비 파일 하나의 앞뒤(병합 기준점 쪽·HEAD 쪽). 비교를 보일 수 없으면 viewable false 와 이유다. */
export interface BaseFileView { path: string; kind: BaseChangeEntry['kind']; before: string; after: string; viewable: boolean; reason?: string; reasonI18n?: WireText }
/** 'branch-view'·'branch-change' 요청의 응답. 요청 종류에 따라 필요한 칸만 채운다. */
export interface BranchResult {
  /** 지금 브랜치(detached 면 없고 commit 만) */
  branch?: string;
  commit?: string;
  branches?: BranchInfo[];
  baseChanges?: BaseChanges;
  baseFile?: BaseFileView;
  /** 바꾸기 결과 안내 글 */
  done?: string;
}

/**
 * 합치기 미리보기(docs/notes/worktree-2026-10-02.md D4, 'worktree-merge-view' merge 의 응답). error 가 있으면 합칠 곳이 없는 것이다.
 * problems 는 원본 쪽 조건이 안 맞는 이유이고, 하나라도 있으면 실행하지 않는다.
 */
export interface WorktreeMergePreview {
  error?: string;
  /** error 의 앱 고정 문구 전송 키(Git 원문이 섞인 error 에는 없다) */
  errorI18n?: WireText;
  /** 기준 브랜치 이름 */
  base: string;
  /** 작업트리 HEAD 전체 OID(실행 요청의 expectHead) */
  head: string;
  files: BaseChangeEntry[];
  dropped: number;
  /** 작업트리의 커밋 안 한 변경 수 */
  dirty: number;
  /** 기준이 병합 기준점 뒤로 앞선 커밋 수 */
  behind: number;
  /** 작업트리가 기준보다 앞선 커밋 수 */
  ahead: number;
  /** 원본의 지금 브랜치(detached 면 없음) */
  sourceBranch?: string;
  problems: string[];
  problemsI18n?: WireText[];
  /** 커밋 글 제안 */
  message: string;
}

export type { CopyApplyFileResult, CopyApplyItem, CopyApplyPreview, CopyChangeResult, CopyCleanPreview } from './isolated-copy-types';

/** 저장소별 작업트리 설정(docs/notes/worktree-2026-10-02.md D3). 열쇠는 원본 최상위 실제 경로다. */
export interface WorktreeIncludeApproval { hash: string; approvedAt: string; count: number; bytes: number }
export interface WorktreeSettings { sourceRoot: string; setupCommand?: string; includeApproval?: WorktreeIncludeApproval }

/** 'worktree-prep-view'·'worktree-prep' 요청의 응답. 요청 종류에 따라 필요한 칸만 채운다. */
export interface WorktreePrepResult {
  settings?: WorktreeSettings;
  preview?: WorktreeIncludePreview;
  /** 준비 명령 출력 끝부분 */
  output?: string;
  /** 'env' 요청: 그 폴더가 작업트리 안이면 줄 환경 변수 */
  env?: Record<string, string>;
}

/** 엔진 터미널의 훅 상태(같은 문서 4번). working 작업 중, waiting 입력 대기, asking 승인 대기, ended 닫힘. */
export type EngineTerminalState = 'working' | 'waiting' | 'asking' | 'ended';
export interface EngineTerminal {
  /** CLI 대화 번호. Claude·Grok은 앱이 정해 넘기고, Codex는 훅이 알린다(같은 문서 3번). */
  cliSession?: string;
  /** CLI가 그 번호로 대화를 시작했다(첫 요청 훅). 이어 열기는 이때만 한다. */
  started?: boolean;
  state?: EngineTerminalState;
  stateAt?: string;
  /** 마지막 요청의 첫 줄 */
  lastPrompt?: string;
  /** 마지막으로 턴이 끝난 시각(확인할 일의 결과) */
  doneAt?: string;
  /** 마지막으로 CLI를 띄운 시각(Codex 대화 번호 찾기) */
  launchedAt?: string;
}

/** 조사·생성·쓰기·실제 확인 네 모드의 제한 시간은 1~120분이다. 조사·생성 기본 15, 쓰기 30, 실제 확인(check) 30(docs/notes/delegation-native-2026-10-05.md 재설계 논쟁 6번)
 * 조사·생성 위임의 제한 시간(분) 기본, 실제 확인(check) 기본, 둘의 상한(docs/notes/delegation-native-2026-10-05.md F6). */
export const DELEGATION_TIMEOUT_DEFAULT = 15;
export const CHECK_TIMEOUT_DEFAULT = 30;
export const DELEGATION_TIMEOUT_MAX = 120;
// 쓰기 위임 상한 상수는 delegation-write-limits.ts 에 있고, 계약 이름은 그대로 다시 내보낸다.
export { WRITE_ATTEMPTS_MAX, WRITE_TIMEOUT_DEFAULT, WRITE_TIMEOUT_MAX, WRITE_IGNORED_NEW_MAX, WRITE_OPS_MAX, WRITE_FILES_MAX } from './delegation-write-limits';
/** 명령 위임 동의 문구의 판 번호. 문구가 바뀌면 올린다(docs/notes/claude-background-2026-10-01.md 9번). */
export const COMMAND_CONSENT_VERSION = 1;
/** 위임이 이만큼(초) 넘게 돌면 지연으로 본다. 앱 위임 7분, 명령 위임 30분(docs/notes/claude-background-2026-10-01.md 6번). 뒤쪽 확인할 일과 카드가 같이 쓴다. */
export const DELEGATION_LONG_SECONDS = 7 * 60;
export const COMMAND_LONG_SECONDS = 30 * 60;
/** 제한 시간 중 이 비율이 지나면 오래 걸린다고 알린다(화면 카드와 확인할 일이 같이 쓴다). */
export const DELEGATION_WARN_RATIO = 0.8;

/** 메뉴 편집의 찾기·다음 찾기·이전 찾기(같은 문서 2번). */
export type FindAction = 'open' | 'next' | 'previous';
/**
 * 앱 메뉴가 화면에 넘기는 명령(docs/notes/tab-shortcuts-2026-09-25.md 1·3·4번). 목록과 번호는 명령 고정 정보 표(commands.ts)에서 나온다
 * (docs/notes/command-registry-2026-10-04.md 1a). 쓰는 곳은 이 이름 그대로 가져간다.
 */
export { APP_COMMANDS, MAIN_COMMANDS } from './commands';
export type { MainCommandId } from './commands';
export type { TurnBrowser, TurnBrowserStep } from './preview-agent';
/**
 * Codex 내장 하위 에이전트 한 개의 기록(docs/notes/native-agent-cards-2026-10-06.md). 앱 위임(Delegation)과 달리 앱이 실행·취소를 쥐지 않으므로
 * 별도 자료로 두고 요청(Turn)에 붙인다. id 는 엔진이 준 자식 대화 번호다.
 * state: starting 시작 알림만 받음, running 실행 중, completed 자식 완료 사건을 받음, failed 실패, interrupted 중단, closed 부모가 닫음,
 * unconfirmed 부모 요청이 끝날 때까지 완료 사건을 받지 못함(끝났는지 모른다).
 */
export interface NativeAgent {
  id: string;
  parentThreadId?: string;
  /** 띄움 호출 번호와 하위 에이전트 경로·작업 이름 */
  callId?: string;
  path?: string;
  /** 부모가 준 지시문(20,000자까지 저장, 화면 상태에는 앞 300자) */
  task?: string;
  /** 부모가 띄울 때 요청한 값. 실제 실행 값이 아니다. */
  requestedModel?: string;
  requestedEffort?: string;
  /** 실행 알림(model/rerouted)이 알린 실제 관측 모델. 알리지 않았으면 없다(미확인). */
  model?: string;
  /** thread/read 의 현재 설정값(마지막 저장값). 실행 관측값이 아니다. */
  configuredModel?: string;
  configuredEffort?: string;
  state: 'starting' | 'running' | 'completed' | 'failed' | 'interrupted' | 'closed' | 'unconfirmed';
  /** 엔진이 알린 실패·중단 사유 */
  error?: string;
  /** 자식의 마지막 답. 부모 답과 합치지 않고 따로 보관한다(200,000자까지 저장, 넘으면 resultTruncated). */
  result?: string;
  resultTruncated?: boolean;
  /** 화면 상태에서만 쓴다: 저장된 결과가 있다(전문은 readDelegation 이 같은 번호로 읽는다). */
  hasResult?: boolean;
  startedAt: string;
  completedAt?: string;
}

export type AppCommand = ScreenCommandId;

export interface SessionSummary extends Pick<SessionSync, 'conflictTitleI18n'>, Partial<Pick<Session, 'createdAt'>> {
  id: string;
  title: string;
  /** 붙은 태그 번호. 없거나 tags 에 없으면 태그 없음. */
  tagId?: string;
  /** AI와 설정하기 대화면 있다. 화면이 실제 승인 방식(묻기)을 판단할 때 쓴다(docs/notes/settings-assistant-2026-10-05.md 고침 확인 2). */
  settingsAssistant?: true;
  /** 임시 대화(docs/notes/temporary-chat-2026-10-03.md T4). */
  temporary?: true;
  /** 임시 대화를 끝내는 중이거나 끝내기에 실패해 새 요청이 막혔다(같은 문서 T7). 화면이 막힘 안내를 보인다. */
  ending?: true;
  /** 예약이 만든 대화(docs/notes/schedules-2026-10-04.md R5). 목록 줄에 예약 표시. */
  scheduled?: { scheduleId: string; name: string };
  /** 이 대화에서 사람이 정한 승인 방식(Session.approvalOverride). summary.approval(다음 요청에 적용될 값)과 구분한다. */
  approvalOverride?: ApprovalMode;
  /** 참여자 대화(docs/notes/participant-runs-2026-10-04.md R4·R7). 목록 줄에 "참여자 실행" 표시. */
  runAs?: { personId: string; name: string };
  /** 열린(해결 안 된) 메시지 댓글 묶음 수(docs/notes/comments-coedit-2026-10-04.md 권고 3). 0 이면 없다. 참여자 투영에도 남긴다. */
  commentCount?: number;
  pinned?: boolean;
  folderId?: string;
  /** 저장된 프로젝트 소속(docs/notes/sidebar-scope-2026-10-02.md S3). 없으면 소속 없음. */
  projectId?: string;
  projectPath: string;
  engine?: EngineId;
  model: string;
  updatedAt: string;
  status: TurnStatus | 'idle';
  /** 지금 실제 승인 방식(모든 대화에 설정 한 값, 없으면 full). */
  approval?: ApprovalMode;
  /** 작업트리 대화면 그 번호와 원본 저장소 최상위. 프로젝트 소속은 원본을 따른다(docs/notes/worktree-2026-10-02.md D2). */
  worktreeId?: string;
  sourceRoot?: string;
  /** 대화 폴더로 만든 대화면 그 경로(칸 머리·삭제 확인 글). */
  chatFolderPath?: string;
  lockedReason?: string;
  /** lockedReason 의 키 + 인자(다국어-6). 없으면 문장을 그대로 보인다. */
  lockedReasonI18n?: WireText;
  /** 보지 않은 결과(완료·실패·결과 불명)가 있다. */
  unread?: boolean;
  /** 갈래 대화면 묶음 위치(사이드바 묶음 표시, docs/notes/rewind-branch-2026-10-02.md D3) */
  branch?: SessionBranch;
  /** 하위 대화면 부모 대화 번호. 부모가 지금 목록에 없으면 싣지 않는다(docs/notes/sub-sessions-2026-10-19.md 4.8절). */
  parentId?: string;
  /** 마지막 요청에 답 안 한 비동기 질문이 있고 그 대화가 도는 중이 아님(백엔드 hasOpenQuestion 과 같은 뜻, 사람이 답할 일). */
  openQuestion?: true;
  /** 다른 기기에서 이어 가는 대화면 그 기기 이름(S7 사이드바 표시). 충돌 사본이면 syncConflict. */
  syncForeign?: string;
  syncConflict?: true;
  /** 자동으로 실행할 대기 수와 멈춘 대기 수 */
  queued?: number;
  held?: number;
  /** 문맥을 줄이는 중이다(docs/notes/compaction-2026-09-24.md 6번). 넘길 요약을 만드는 중이면 summarizing도 참이다. */
  compacting?: boolean;
  summarizing?: boolean;
  /** 확인 대기가 승인이 아니라 엔진의 질문이다(docs/notes/user-question-2026-09-24.md 2번). */
  asking?: boolean;
  /**
   * 지금 살아 있는 작업 중 가장 이른 시작(ISO). 마지막 요청이 running·stopping 이면 그 createdAt, running 압축·위임의 createdAt 중 최소다.
   * 승인 대기 요청은 넣지 않는다. 요청이 끝나고 위임만 남으면 늦어질 수 있어 연속 작업 시간이 아니다. 없으면(옛 호스트 포함) 사이드바는 상대 시각을 쓴다.
   */
  workingSince?: string;
  /** 뒤에서 도는 위임 수(docs/notes/delegation-2026-09-24.md 7번) */
  delegating?: number;
  /** 그중 명령 방식 수(사이드바 문구용) */
  delegatingCommands?: number;
  /** 엔진 터미널 대화의 훅 상태(docs/notes/terminal-slice2-2026-09-27.md 5번) */
  terminal?: { state?: EngineTerminalState; started?: boolean; doneAt?: string };
  /** 옛 Lair에서 가져온 대화다(docs/notes/perf-2026-10-01.md 4절). 설정의 "가져온 대화 경로 맞추기" 단추를 보일지 정한다. */
  fromLair?: true;
}

export interface WorkspaceState {
  path: string | null;
  suggestedPath: string;
  pendingPath: string | null;
  restartRequired: boolean;
  problem?: string;
  /** 앱 작업 폴더가 앱 설치 위치 안이다(docs/notes/folder-paths-2026-10-01.md 9번). 새 판으로 바꿀 때 함께 지워질 수 있다. */
  insideApp?: boolean;
}

/** 모델을 부르지 않고 판정한 Codex 준비 상태(docs/notes/first-run-2026-09-23.md). */
export type EngineState = 'checking' | 'ready' | 'missing' | 'unsupported-version' | 'login-required' | 'usage-limit' | 'unknown';

export interface EngineInfo {
  /** 보내기를 막지 않는 상태인가. 확인 중·미설치·지원하지 않는 버전·로그인 필요면 false. */
  available: boolean;
  state: EngineState;
  detail: string;
  /** detail 의 키 + 인자(다국어-6). 화면은 wireLocal 로 그린다. 없으면 detail 문장. */
  detailI18n?: WireText;
  /** 마지막 로그아웃 결과 한 줄(detail 에 붙던 글). 서비스가 채운다. */
  logoutNote?: string;
  logoutNoteI18n?: WireText;
  version?: string;
  minimumVersion: string;
  account?: { kind: 'chatgpt' | 'claude' | 'grok' | 'api-key' | 'bedrock' | 'other' | 'none'; email?: string; plan?: string };
  /** 엔진이 알린 고를 수 있는 모델(Codex 모델 목록, Claude 초기화 응답). 없으면 엔진 연결의 기본 목록을 쓴다. */
  models?: EngineModel[];
  /** 새 대화의 기본 모델(스냅숏에서 채운다). */
  defaultModel?: string;
  /** 엔진 연결의 지원 능력(스냅숏에서 채운다). */
  capabilities?: EngineCapabilities;
  /** 엔진이 알린 사용 한도 해제 예정 시각. 없으면 모름. */
  limitResetsAt?: string;
  checkedAt?: string;
  /** 엔진이 알린 구독 한도의 창별 사용 비율(docs/notes/limits-2026-09-24.md). 받은 적이 없으면 없다. */
  limits?: EngineLimits;
  /** 앱 전용 폴더에서 로그인하는 엔진이다(Grok, 스냅숏에서 채운다). 거짓이면 이 컴퓨터의 엔진 로그인을 그대로 쓴다. */
  appLogin?: boolean;
  /**
   * 터미널에서 로그인하는 명령(ACP 엔진, docs/notes/acp-engines-2026-10-05.md 설계 4절). 앱은 구독 로그인을 대신 들고 있지 않고
   * 엔진 CLI 의 로그인을 쓴다. 화면은 로그인이 필요할 때 이 명령을 복사할 수 있게 보인다.
   */
  loginCommand?: string;
  /** 진행 중이거나 실패한 앱 로그인. 끝나면 없어지고 상태 확인이 다시 돈다(docs/notes/grok-engine-2026-09-24.md 설계 6번). */
  login?: EngineLoginView;
  /** 못 찾았거나 지정 경로가 잘못됐을 때의 후보 진단(docs/notes/engine-install-2026-09-29.md 3절 6·7번). */
  locate?: EngineLocateView;
  /** 공식 설치기가 돌아가는 중(같은 문서 4절). */
  install?: EngineInstallView;
  /** 찾은 설치본의 종류·경로·올리는 방법(docs/notes/engine-update-2026-10-03.md U2). */
  source?: EngineSourceView;
  /** 최신 판 조회 결과(같은 문서 U3). 조회 전이거나 조회하지 않는 설치본이면 없다. */
  latest?: EngineLatestView;
  /**
   * 쓰는 실행 파일(docs/notes/engine-binary-path-2026-10-12.md D7). auto 는 자동으로 찾은 실제 실행 파일, setting 은 화면에서 저장한 지정(고른 그대로의 경로),
   * env 는 환경 변수 지정. setting·env 인데 locate.explicitUnusable 이면 지정한 경로가 실패한 상태다(path 는 지정한 경로). 못 찾았으면 없다.
   */
  binaryPath?: EngineBinaryPath;
  /** 마지막 업데이트 결과 한 줄. 다시 확인이나 앱 재시작 때 없어진다. */
  update?: EngineUpdateNote;
  /** 판 출력 원문의 판(꼬리표 포함, 0.159.0-alpha 등). version 은 숫자 세 자리만 둔다. */
  versionFull?: string;
  /** Grok 이 판 출력에 적은 채널. */
  channel?: 'stable' | 'alpha';
  /** 추가 계정과 그 상태(스냅숏에서 채운다, docs/notes/accounts-2026-09-26.md 11번). */
  accounts?: EngineAccountView[];
  /** Linux Codex 격리 초기화 시험 결과(docs/notes/linux-codex-sandbox-2026-10-09.md). 성공했거나 시험하지 않는 환경이면 없다. */
  sandbox?: EngineSandboxNote;
}

/** 엔진이 쓰는 실행 파일과 그 출처(EngineInfo.binaryPath). */
export interface EngineBinaryPath {
  path: string;
  from: 'auto' | 'setting' | 'env';
}

/**
 * 실행 파일 직접 지정 거절 이유(요청 engine-path). 화면은 reason 으로 분기하고 문구는 text·textI18n 을 그린다.
 * missing 없음, directory 폴더·.app 묶음, broken-link 링크 끊김, not-executable 실행 권한 없음, windows-exe Windows 는 .exe 필요(셈 풀기 실패),
 * wrong-program 판·이름 표지를 못 읽음, too-old 최소 판 미만, installing 설치·업데이트 중, env-locked 환경 변수로 지정돼 바꿀 수 없음,
 * not-absolute 다듬은 뒤에도 절대 경로가 아님(전체 경로를 넣어야 함), write-failed 지정 파일을 쓰지 못함(저장 안 됨).
 */
export type EnginePathReason = 'missing' | 'directory' | 'broken-link' | 'not-executable' | 'windows-exe' | 'wrong-program' | 'too-old' | 'installing' | 'env-locked' | 'not-absolute' | 'write-failed';

/** 거절된 엔진 경로 지정. 저장하지 않았고 앞 값이 그대로다. */
export interface EnginePathRejected {
  rejected: true;
  engine: EngineId;
  reason: EnginePathReason;
  text: string;
  textI18n?: WireText;
}

/**
 * 엔진 경로 지정 요청의 답. 거절이 아니면 갱신된 스냅숏(check-engine 응답과 같은 모양)이고, warning 은 저장은 했지만 알릴 것이 있을 때만 싣는다
 * (version-folder: 판 폴더 안의 경로라 업데이트 뒤 끊길 수 있음).
 */
export type EnginePathResult = (Snapshot & { rejected?: undefined; warning?: { reason: 'version-folder'; text: string; textI18n?: WireText } }) | EnginePathRejected;

/** Codex 격리(bwrap)를 세우지 못했거나 확인하지 못했다는 안내. 보내기는 막지 않는다. */
export interface EngineSandboxNote {
  /** blocked: 시험이 격리 초기화 실패로 끝남. unchecked: 시간 초과·하위 명령 없음 등으로 판정하지 못함. */
  state: 'blocked' | 'unchecked';
  /** apparmor-userns: 사용자 네임스페이스 제한으로 확정, apparmor-userns-maybe: 제한이 켜져 있으나 출력이 일반적임,
   * bwrap-missing: 실행 PATH 에 bwrap 이 없음, bwrap-path: PATH 의 첫 bwrap 이 /usr/bin/bwrap 이 아님,
   * existing-policy: bwrap 에 붙는 AppArmor 정책이 이미 있음, other: 그 밖 */
  cause: 'apparmor-userns' | 'apparmor-userns-maybe' | 'bwrap-missing' | 'bwrap-path' | 'existing-policy' | 'other';
  text: string;
  textI18n?: WireText;
  /** 이 컴퓨터에서 직접 실행할 복구 명령(앱은 실행하지 않는다). 원격 전달에는 싣지 않는다. */
  commands?: string[];
}

/**
 * 앱 로그인 칸. waiting이면 브라우저에서 주소를 열어 코드를 확인하는 중이다. input이 code면 브라우저에 나온 코드를 붙여 넣어 받고
 * (Claude 추가 계정, docs/notes/accounts-2026-09-26.md 6번), submitting은 받은 코드를 엔진에 넘겨 확인하는 중이다.
 */
export interface EngineLoginView {
  state: 'starting' | 'waiting' | 'submitting' | 'failed';
  input?: 'code';
  url?: string;
  code?: string;
  expiresAt?: string;
  error?: string;
  errorI18n?: WireText;
  /** failed 의 종류(agy 앱 안 로그인). no-url·no-pty 는 앱 안 로그인이 안 되니 "터미널에서 로그인" 길을 보인다. expired 는 10분 만료, exited 는 로그인 전에 agy 가 끝남. 없으면 일반 실패. */
  failure?: 'no-url' | 'no-pty' | 'expired' | 'exited';
}

import type { EngineLimits, LimitWindow, ResetCreditResult } from './limit-types';
export type { EngineCredits, EngineLimits, EngineResetCredits, LimitWindow, ResetCreditOutcome, ResetCreditResult } from './limit-types';

/**
 * 엔진 연결이 실제로 지원하는 것(docs/notes/engine-capabilities-2026-09-24.md). 실측으로 정한 것만 참이다.
 * 불변식: 실행부의 판단(끼워 넣기)과 화면 표시가 같은 값을 읽는다. 깨지면 화면은 지원한다고 보이는데 실행부는 거절하거나 그 반대가 된다.
 */
export interface EngineCapabilities {
  /** 그림 첨부를 엔진에 그림으로 넘긴다. 아니면 파일 목록으로만 알린다. */
  images: boolean;
  /** 파일 변경·명령을 요청마다 앱이 승인받는다. */
  approvals: boolean;
  /** 실행 중 추가 지시를 지금 실행에 넣는다. 아니면 다음 실행 대기로 둔다. */
  steer: boolean;
  /** 돌고 있는 요청에 첨부를 함께 끼워 넣을 수 있다. Codex turn/steer 의 UserInput(B-76). */
  steerAttachments?: boolean;
  /** 중지를 엔진이 확인한다. */
  stop: boolean;
  /** 같은 엔진 대화를 이어 간다. */
  resume: boolean;
  /** 명령 격리 방식(실행 기록에도 남긴다). */
  sandbox: 'read-only' | 'none';
  /** 입력칸 "/compact"로 엔진 대화의 문맥을 줄인다(docs/notes/compaction-2026-09-24.md). */
  compaction: boolean;
  /** 엔진을 바꿀 때 넘길 요약을 만든다(docs/notes/summary-handoff-2026-09-26.md 2·9번). */
  summary: boolean;
  /** 엔진의 질문에 답하기(docs/notes/user-question-2026-09-24.md). Claude만 true, Codex·Grok은 아직 false다. */
  questions: boolean;
  /** 읽기 전용 위임 작업자로 돈다(docs/notes/grok-worker-2026-09-26.md 9번). 설정의 위임 대상은 이 엔진들에서 고른다. */
  worker: boolean;
  /** 작업자가 할 수 있는 모드(docs/notes/role-policy-2026-10-05.md 논쟁 6·11번). Codex 는 조사만, Windows 의 Grok 은 쓰기 없음. check(실제 확인)는 지원하는 엔진만 넣는다. worker 가 거짓이면 없다. */
  workerModes?: WorkerMode[];
  /** 작업자 모드를 못 여는 까닭(엔진이 승인·읽기 전용을 강제하지 못함). 막힌 모드가 있을 때만 있다. */
  workerNote?: string;
  /** 앱 데이터 폴더의 계정별 폴더에 추가 계정을 두고 대화마다 고른다(docs/notes/accounts-2026-09-26.md 1번). */
  accounts: boolean;
  /**
   * 조사·계획 모드 지원과 못 하는 까닭(docs/notes/plan-mode-2026-10-02.md D2). 없으면 둘 다 지원하지 않는다(읽기 전용을 확인하지 못했다).
   * 실행 모드는 늘 지원한다. 실행부의 거절과 화면 표시가 같은 값을 읽는다.
   */
  modes?: { research?: ModeSupport; plan?: ModeSupport };
  /** 웹 검색 끄기와 사이트 범위(docs/notes/web-sources-2026-10-03.md W1). 없으면 앱이 끄기·범위를 맡기지 못한다. */
  web?: { off: boolean; sites: WebSiteControl };
  /** 보낸 글 취소가 엔진 대화를 되감는 단위: message 메시지 번호, turn 요청 단위. 없으면 되감지 못해 결속을 지운다. */
  rewind?: 'message' | 'turn';
}
/** 사이트 범위를 엔진이 강제하는 정도. 정의는 근거 타입과 함께 evidence-types.ts 에 있다(순환 import 방지). */
export type { WebSiteControl } from './evidence-types';
/** 엔진의 모드 하나 지원. reason은 지원하지 않을 때 사용자에게 보일 까닭이다. */
export interface ModeSupport { supported: boolean; reason?: string }

/**
 * 추가 계정 하나(docs/notes/accounts-2026-09-26.md 3번). 비밀값은 싣지 않는다. 기본 계정(엔진이 지금 쓰는 로그인)은 목록에 없다.
 * 계정 폴더는 저장하지 않고 엔진과 번호로 계산한다(4번).
 */
export interface EngineAccount {
  id: string; engine: EngineId; name: string; createdAt: string;
  /** 연결 계정(docs/notes/provider-accounts-2026-10-03.md D1). 없으면 구독 로그인 계정이다. 비밀(키)은 담지 않는다. */
  provider?: ProviderConfig;
  /** 저장소에서 읽은 provider 칸이 틀렸다. 연결 계정으로 쓰지 못하고 상태는 unknown이다(저장하지 않는 읽기 표식). */
  providerInvalid?: true;
  /** 한도 뒤 크레딧 사용 허용(docs/notes/codex-credits-2026-11-03.md) */
  creditUse?: true;
}

/** 엔진 로그인(기본 계정)의 표시 이름과 Lair 에서 제거한 표식(docs/notes/settings-accounts-2026-10-08.md 3번). 인증·사용자 HOME 은 건드리지 않는다. */
export interface EngineLoginSetting {
  name?: string; removed?: true;
  /** 한도 뒤 크레딧 사용 허용(docs/notes/codex-credits-2026-11-03.md) */
  creditUse?: true;
}

/** 연결 계정 설정(같은 문서 D1·논쟁 4·6). 만든 뒤 kind·baseUrl·auth·api는 바꾸지 않는다. */
export type ProviderKind = 'api-key' | 'custom' | 'local';
export interface ProviderConfig {
  kind: ProviderKind;
  baseUrl?: string;
  /** Claude custom만(기본 bearer) */
  auth?: 'bearer' | 'x-api-key';
  /** Grok custom만(기본 chat_completions) */
  api?: 'chat_completions' | 'responses' | 'messages';
  models: string[];
  needsKey: boolean;
}

/** 주 프로세스 → 백엔드: 연결 계정 키 묶음(늘 전체 묶음, 백엔드 메모리에만 둔다). */
export interface ProviderKeysMessage { kind: 'provider-keys'; keys: Record<string, string> }
/** 백엔드 → 주 프로세스: 지금 저장소에 있는 계정 번호 목록(목록에 없는 번호의 키를 지운다). */
export interface ProviderAccountsMessage { kind: 'provider-accounts'; ids: string[] }

// 스킬 목록 타입(SkillEngine·SkillRow·SkillsView 등)은 skill-translate-types.ts 로 옮겼다(이동만, 뜻 변경 없음).
import type { SkillEngine, SkillsView } from './skill-translate-types';
export type { SkillEngine, SkillSourceKind, SkillState, SkillEngineEntry, SkillRow, SkillsEngineStatus, SkillsView } from './skill-translate-types';
import type { SkillTranslatePlan, SkillTranslateResult, SkillTranslateTarget } from './skill-translate-types';
export type { SkillTranslateCode, SkillTranslateLastRun, SkillTranslatePlan, SkillTranslateResult, SkillTranslateTarget, SkillTranslations } from './skill-translate-types';
export interface SkillBody { name: string; path: string; text: string; truncated: boolean }
/** 앱 설정에 저장하는 Lair 끄기 한 줄. key 는 그 엔진의 끄기 키(Claude 스킬 이름, Codex SKILL.md 경로, Grok 스킬 이름). */
export interface SkillOff { engine: SkillEngine; key: string; label: string }
import type { LairCustomSkillsView } from './custom-skill-types';
export type { LairCustomSkillScope, LairCustomSkillCopy, LairCustomSkillRecord, LairCustomSkillSummary, LairCustomSkillsView } from './custom-skill-types';
/** 백엔드 → 주 프로세스: 원격 기기가 넣은 연결 계정 키를 이 컴퓨터 보안 저장소에 저장(빈 값은 지움)해 달라(remote-accounts-mcp-skills D2). */
export interface ProviderKeyStoreMessage { kind: 'provider-key-store'; id: string; accountId: string; value: string }
/** 주 프로세스 → 백엔드: 위 저장의 결과. 실패 이유에 값은 싣지 않는다. */
export interface ProviderKeyStoredMessage { kind: 'provider-key-stored'; id: string; ok: boolean; error?: string }

/** 추가 계정의 상태(같은 문서 11·12번). usedBy는 이 계정을 고른 대화 수다. */
export interface EngineAccountView extends EngineAccount {
  /** 연결 계정에 키가 저장돼 있다(키 자체는 싣지 않는다). */
  hasKey?: boolean;
  available: boolean;
  /** 그 계정의 로그인·로그아웃·지우기·정리가 끝나지 않아 실행부가 그 계정을 쓰지 않는 중이다(docs/notes/browser-follow3-2026-10-04.md 고침 확인 2). */
  authBusy?: boolean;
  state: EngineState;
  detail: string;
  detailI18n?: WireText;
  /** 마지막 로그아웃 결과 한 줄(detail 에 붙던 글). 서비스가 채운다. */
  logoutNote?: string;
  logoutNoteI18n?: WireText;
  email?: string;
  plan?: string;
  models?: EngineModel[];
  limits?: EngineLimits;
  limitResetsAt?: string;
  checkedAt?: string;
  login?: EngineLoginView;
  usedBy: number;
}

/** 첫 실행 안내의 저장된 진행과 지금 판정한 사실. */
export interface FirstRunState {
  finished: boolean;
  /** '나중에'로 닫았다. ready 와 함께 보고 준비 전이면 작성기 위에 남은 조건을 보이고 보내기를 막는다. */
  later: boolean;
  step: 1 | 2 | 3;
  /** 첫 화면에서 고른 길. 옛 기록은 단계·신뢰·첫 요청으로 추정하고, 모르면 null(docs/notes/first-run-fork-2026-09-29.md 2절). */
  route: 'local' | 'remote' | null;
  /** 이 컴퓨터 준비 끝: 기본 엔진을 보낼 수 있고, 신뢰한 작업 폴더가 적용돼 다시 시작 대기·문제가 없고, 기록 저장이 막히지 않음(3절). */
  ready: boolean;
  /** 준비가 계정 때문에 막혔을 때만 이유 구분: 새 대화 기본 없음 / 엔진 로그인 Lair 에서 제거 / 기본 추가 계정 로그인 필요. 계정을 임의로 보충하지 않는다(docs/notes/settings-accounts-2026-10-08.md 4절 F2). */
  accountBlock?: 'no-default' | 'engine-login-removed' | 'account-login-required';
  trustedRoot: string | null;
  /** 신뢰한 경로가 지금 적용된 작업 폴더와 같은가 */
  trusted: boolean;
  firstRequest: { sessionId: string; turnId: string; title: string; status: TurnStatus | 'missing' } | null;
}

/** 프로젝트 지시 파일 이름(같은 문서 9번). 앞의 둘은 늘 보이고 나머지는 있을 때만 보인다. */
export const PROJECT_RULE_NAMES = ['CLAUDE.md', 'AGENTS.md', 'CLAUDE.local.md', '.claude/CLAUDE.md', 'AGENTS.override.md'] as const;
export type ProjectRuleName = typeof PROJECT_RULE_NAMES[number];
/** 지시 파일 목록의 한 줄. engine은 그 파일을 읽는 엔진, size는 있을 때의 바이트 수, link는 링크면 실제 파일 경로다. */
export interface ProjectRuleItem { name: ProjectRuleName; engine: EngineId; exists: boolean; size?: number; link?: string; problem?: string; problemI18n?: WireText }
export type ProjectRulesAction =
  | { kind: 'list'; projectId: string }
  | { kind: 'read'; projectId: string; name: ProjectRuleName }
  /** 없는 파일은 빈 파일의 판으로 읽고, 그 판으로 저장하면 새로 만든다(같은 문서 9번). */
  | { kind: 'write'; projectId: string; name: ProjectRuleName; version: string; text: string };
export interface ProjectRulesResult { files?: ProjectRuleItem[]; view?: RuleFileView; save?: RuleFileSave }

/**
 * 변경 보기(docs/notes/projects-slice2-2026-09-27.md). 받지 않은 변경은 작업 파일과 스테이지, 받은 변경은 스테이지와 마지막 커밋의 차이다.
 * path는 저장소 맨 위 기준 경로('/' 구분), rel은 연 폴더 기준 표시 경로다.
 */
export type ChangeGroup = 'unstaged' | 'staged';
export interface ChangeEntry {
  path: string;
  rel: string;
  group: ChangeGroup;
  kind: 'added' | 'deleted' | 'modified' | 'conflict';
  added?: number;
  removed?: number;
  binary?: boolean;
  /** 추적하지 않는 새 파일(스테이지에 없음) */
  untracked?: boolean;
  submodule?: boolean;
  /**
   * 화면 표시 글자(화면개선-4 F): M 수정, U 추적 안 하는 새 파일, A 받은 새 파일, D 삭제, R 이름 바꿈, ! 충돌. 무시는 목록이 아니라 check-ignored 로 묻는다.
   * 파일 단위만 싣고 폴더 상태는 화면이 모은다.
   */
  mark?: 'M' | 'U' | 'A' | 'D' | 'R' | '!';
  /** 이름 바꿈(mark R)의 옛 경로(path 와 같은 저장소 기준)와 옛 표시 경로(rel 과 같은 기준). 줄 수는 두 경로를 이은 한 줄이다. */
  oldPath?: string;
  oldRel?: string;
  /**
   * 내용 판 표시. 받지 않은 변경은 작업 파일의 수정 시각·크기, 받은 변경은 스테이지의 Git 개체 번호다. 줄 수가 같은 바깥 수정도
   * 목록이 달라진 것으로 알아채는 데 쓴다(docs/notes/windows-qa-2026-09-27.md B3).
   */
  stamp?: string;
}
export interface ChangesList {
  /** missing: Git이 없음, none: Git 저장소가 아닌 폴더, ignored: 상위 저장소가 무시하는 폴더, ok: 목록, broad: 넓은 범위 폴더라 조회 안 함 */
  git: 'missing' | 'none' | 'ignored' | 'ok' | 'broad';
  branch?: string;
  /** 따라가는 원격 브랜치와 앞선·뒤처진 커밋 수(마지막으로 받아 온 원격 기준, docs/notes/projects-slice4-2026-09-27.md) */
  upstream?: string;
  ahead?: number;
  behind?: number;
  /** 원격 저장소 이름들 */
  remotes?: string[];
  entries: ChangeEntry[];
  /** 한도(500개)를 넘어 빠진 수 */
  dropped: number;
}
export interface ChangeView {
  path: string;
  /** 작업 파일의 절대 경로 */
  abs: string;
  group: ChangeGroup;
  kind: ChangeEntry['kind'];
  /** 이름 바꾼 받은 변경이면 옛 경로(비교 기준이 그 파일의 마지막 커밋 판이다) */
  oldPath?: string;
  /** 비교 기준(받지 않은 변경은 스테이지, 받은 변경은 마지막 커밋)과 바뀐 쪽(작업 파일 또는 스테이지). 없으면 빈 글로 본다. */
  original: string;
  current: string;
  /** 스테이지 blob 번호와 작업 파일 내용 해시. 쓰기 전에 이 판인지 확인한다. */
  indexVersion: string;
  fileVersion: string;
  /** 비교를 보일 수 있는지(텍스트·2 MiB 이하·필터 없음), 덩어리 받기·되돌리기와 고치기를 할 수 있는지 */
  viewable: boolean;
  chunkable: boolean;
  /** 못 하는 이유 */
  reason?: string;
  /** reason 의 키 + 인자(다국어-6). 없으면 reason 문장을 그대로 보인다. */
  reasonI18n?: WireText;
}
export type ChangesAction =
  | { kind: 'list'; folder: string }
  /** 트리 Git 표시용 가벼운 목록(git status 한 번, 줄 수 없음). 결과는 list와 같은 모양이다. */
  | { kind: 'list-marks'; folder: string }
  /** 화면에 보이는 경로(연 폴더 기준, 폴더는 끝에 '/')가 Git 무시 대상인지 묻는다. 200개까지, 연 폴더 밖·링크를 지나는 경로는 무시로 치지 않는다. 결과는 ChangesResult.ignored. */
  | { kind: 'check-ignored'; folder: string; paths: string[] }
  | { kind: 'read'; folder: string; path: string; group: ChangeGroup; versionsOnly?: boolean }
  /** 덩어리 받기·받기 취소: 스테이지 내용을 text로 바꾼다. version은 읽은 스테이지 blob 번호다. */
  | { kind: 'write-index'; folder: string; path: string; text: string; version: string }
  /** 덩어리 되돌리기·고친 글 저장: 작업 파일을 text로 바꾼다. version은 읽은 작업 파일 해시다. */
  | { kind: 'write-file'; folder: string; path: string; text: string; version: string }
  /** stage-file의 version: 비교를 본 작업 파일 판. 주면 지금 판과 같을 때만 받는다(windows-qa B3). */
  | { kind: 'stage-file' | 'unstage-file' | 'revert-file'; folder: string; path: string; version?: string }
  /** 외부 편집기로 열 파일의 절대 경로(폴더 안인지 확인) */
  | { kind: 'locate'; folder: string; path: string }
  /** 연 폴더 안 바뀐 파일을 모두 받는다(새 파일 포함, .gitignore는 따름) / 받은 변경을 모두 받기 취소한다 */
  /** versions: 비교를 본 파일(저장소 기준 경로)과 그 작업 파일 판. 하나라도 지금 판과 다르면 모두 받지 않는다(docs/notes/windows-qa-2026-09-27.md D2). */
  | { kind: 'stage-all'; folder: string; versions?: Record<string, string> }
  | { kind: 'unstage-all'; folder: string }
  /** 이번 턴 모두 받기용 일괄 받기. versions 는 비교를 본 파일(연 폴더 기준 경로)과 그 작업 파일 판이다. 파일마다 그 판이 지금과 다르면 그 파일만 받지 않고 stale 에 담는다. 결과는 list 와 stagedFiles. */
  | { kind: 'stage-files'; folder: string; paths: string[]; versions?: Record<string, string> }
  /** 받은 변경(스테이지)만 커밋한다. 연 폴더 밖에 받은 변경이 있으면 거절한다. */
  | { kind: 'commit'; folder: string; message: string }
  /** 지금 브랜치를 따라가는 원격으로 푸시한다. 따라가는 원격이 없으면 원격이 하나일 때 같은 이름으로 올리고 따라가게 한다. */
  | { kind: 'push'; folder: string;
    /** 화면이 확인창에 보인 대상. 없으면 위 동작(폰 화면). urls 는 보낼 주소(사용자 정보 지운 꼴로 실행 직전 다시 읽어 견준다). */
    target?: { remote: string; branch: string; oid: string; urls: string[] } }
  /** 원격 현황(주소·추적·푸시 대상·앞섬/뒤처짐·마지막 확인). 이름이 read 로 시작해 보기 묶음이다. 결과는 ChangesResult.remote. */
  | { kind: 'read-remote'; folder: string; light?: boolean }
  /** 머리 배지용 가벼운 읽기(올릴 수·받을 수만). 결과는 ChangesResult.badge. */
  | { kind: 'read-badge'; folder: string }
  /** 추적 원격과 푸시 원격을 받는다(지우지 않음). 결과는 fetched 또는 gitFailure. */
  | { kind: 'fetch'; folder: string }
  /** 추적 브랜치를 빨리감기로만 합친다. expect 는 확인창에 보인 HEAD·추적 참조 해시다. 결과는 pulled 또는 gitFailure. */
  | { kind: 'pull'; folder: string; expect: { head: string; upstream: string } }
  | HistoryAction;
/**
 * 결과물 판(docs/notes/result-versions-2026-10-03.md D1·D2). 파일 하나의 판 기록을 되돌리기 스냅숏에서 읽는다. list-·read- 이름이라 보기 묶음이고 version-attach 는 변경이다.
 * path 는 folder 기준 상대 경로('/' 구분, .. 없음)다. 마지막 이름은 지운 파일의 옛 판을 위해 없어도 된다. id·base 는 앱 기록의 체크포인트 번호이거나 'now'(지금 디스크 내용)이고,
 * 실행부가 그 번호가 이 파일의 판인지 다시 확인한다.
 */
export type FileVersionAction =
  | { kind: 'list-file-versions'; folder: string; path: string }
  /** base 가 없으면 그 판의 직전 기록과 견준다. */
  | { kind: 'read-file-version'; folder: string; path: string; id: string; base?: string }
  /** 고른 판의 바이트 사본을 그 대화의 첨부로 만든다(지금과 같아도 사본). */
  | { kind: 'version-attach'; folder: string; path: string; id: string; sessionId: string }
  /** 그림 판 나란히 보기(docs/notes/image-results-2026-10-03.md D4). 보기 묶음. */
  | { kind: 'read-file-version-image'; folder: string; path: string; id: string; base?: string }
  /** Codex 생성 그림을 대화 작업 폴더에 덮어쓰지 않는 새 이름으로 저장(같은 문서 D2). 변경 묶음. */
  | { kind: 'save-generated-image'; sessionId: string; turnId: string; path: string }
  /** Codex 생성 그림의 바이트 사본을 대상 대화 첨부로(같은 문서 D2). 변경 묶음. */
  | { kind: 'generated-attach'; sessionId: string; turnId: string; path: string; targetSessionId: string }
  /** 보관 사본 첨부를 대상 대화의 새 첨부로. 변경 묶음 */
  | { kind: 'attachment-attach'; sessionId: string; attachmentId: string; targetSessionId: string }
  /** 브라우저 동작 기록을 HTML 파일 하나로 그 대화 작업 폴더에 새 이름으로 저장(docs/notes/browser-follow2-2026-10-04.md R6). 이 컴퓨터의 Lair 앱만. 결과는 saved.path·name */
  | { kind: 'save-browser-record'; sessionId: string; turnId: string };
export type { VersionImageSide, FileVersionKind, FileVersionState, FileVersionFlag, FileVersionSource, FileVersionLine, FileVersionNow, FileVersionList, FileVersionView } from './file-version-types';
/**
 * Git 기록 화면의 읽기 동작(docs/notes/github-pr-2026-10-02.md D7). 이름이 list·read 로 시작해 보기 묶음이다(request-gate readAction).
 * oid 는 16진 40·64자, path 는 폴더 안 상대 경로('/' 구분, .. 없음)다. 실행부가 다시 확인한다.
 */
export type HistoryAction =
  /** 기록 목록 한 쪽. oid 가 없으면 지금 HEAD 를 고정해 돌려주고, 다음 쪽부터 화면이 그 oid 를 넘긴다. */
  | { kind: 'list-history'; folder: string; oid?: string; skip: number;
    /** 보는 범위. 없으면 head(지금 동작). tips 가 있으면 그 해시들에서 읽고(16진 해시만), 없으면 scope 로 시작 참조를 풀어 tips 로 돌려준다. */
    scope?: (typeof import('./schedule-lists').GIT_HISTORY_SCOPES)[number]; tips?: string[];
    /** related 에 더할 작업트리 기준 브랜치 이름(refs/heads/<base>, 없으면 뺀다) */
    base?: string;
    /** 참이면 git log·marks 를 읽지 않고 newer·refsMoved 만 돌려준다(보는 목록을 바꾸지 않는 확인) */
    probe?: boolean }
  /** 커밋 상세와 바뀐 파일 목록. parent 가 없으면 첫 부모(첫 커밋은 빈 트리)와 견준다. */
  | { kind: 'read-commit'; folder: string; oid: string; parent?: string }
  /** 커밋의 파일 하나 차이. path 는 그 커밋의 경로, oldPath 는 이름이 바뀐 파일의 부모 쪽 경로(저장소 맨 위 기준) */
  | { kind: 'read-commit-file'; folder: string; oid: string; parent?: string; path: string; oldPath?: string }
  /** 파일 하나를 바꾼 커밋들(git log --follow, 200개까지) */
  | { kind: 'list-file-history'; folder: string; path: string };
/** git-settings-check 결과. 출력 원문·주소는 싣지 않는다. */
export interface GitSettingsCheck {
  host: string;
  /** hostkey: 이 컴퓨터가 아직 그 호스트의 SSH 키를 모름, hostkey-changed: 알던 키와 달라짐 */
  result: 'ok' | 'auth' | 'hostkey' | 'hostkey-changed' | 'network' | 'missing' | 'timeout' | 'error';
  transport: 'https' | 'ssh';
  /** 확인에 쓴 등록 프로젝트 이름 */
  project: string;
  /** SSH 원격이고 hostkey·hostkey-changed 일 때만. 원격 주소의 사용자·포트(주소 전체·경로는 싣지 않는다) */
  ssh?: { user: string; port?: number };
  checkedAt: number;
}
/** git-settings-install 결과 */
export interface GitSettingsInstall {
  tool: 'git' | 'gh';
  plan: { kind: 'dialog'; /** 이미 설치돼 있어 설치 창을 띄우지 않았다 */ installed?: boolean } | { kind: 'terminal'; command: string } | { kind: 'page'; url: string };
}
/** git-settings-ssh-key 결과. file 이 null 이면 공개키가 없다. */
export type GitSettingsSshKey = { file: string; key: string } | { file: null };
/** read-git-settings 의 호스트 한 줄. 서비스 종류는 호스트 이름과 gh 로그인 호스트 여부로 정한다. */
export interface GitSettingsHost {
  host: string;
  service: 'github' | 'github-enterprise' | 'gitlab' | 'bitbucket' | 'azure' | 'other';
  /** 이 호스트를 원격 주소로 쓰는 등록 프로젝트 수 */
  projects: number;
  /** plaintext: gh 가 토큰을 키링이 아닌 평문 설정 파일에 저장함(토큰 값·경로는 싣지 않는다) */
  gh?: { accounts: Array<{ user: string; active: boolean; source: 'gh' | 'env'; plaintext?: boolean }>; state: 'ok' | 'login' | 'error' };
  /** GitHub 계열 호스트만. gh: 전역 자격 도우미 값 중 gh 가 있음, backup: 연결 전 값을 기록해 둠(되돌리기 가능). 값 원문은 싣지 않는다. */
  credential?: { gh: boolean; backup: boolean };
}
/** 커밋 작성자(전역과, 폴더를 주었을 때 그 저장소의 적용값) */
export interface GitSettingsAuthor {
  global: { name?: string; email?: string };
  project?: { folder: string; name?: string; email?: string; localName?: boolean; localEmail?: boolean };
}
/** read-git-settings 결과. 자격 값·토큰·키 지문은 싣지 않는다. part 를 주면 그 부분의 칸만 있다(local: git·https·ssh·author, services: hosts·gh). */
export interface GitSettings {
  git?: { found: boolean; version?: string; path?: string };
  https?: { helper: 'osxkeychain' | 'manager' | 'libsecret' | 'store' | 'cache' | 'wincred' | 'none' | 'other'; plaintext: boolean };
  ssh?: { agent: 'keys' | 'empty' | 'none' | 'unknown'; keys?: number };
  author?: GitSettingsAuthor;
  hosts?: GitSettingsHost[];
  gh?: { found: boolean; version?: string; old?: boolean };
  checkedAt: number;
}
export interface ChangesResult {
  github?: GithubReadResult;
  /** read-git-settings 결과 */
  gitSettings?: GitSettings;
  /** git-settings-author 결과: 바꾼 뒤 다시 읽은 작성자 */
  gitSettingsAuthor?: GitSettingsAuthor;
  /** git-settings-gh 결과: 바꾼 뒤 그 호스트의 gh 로그인 상태(없으면 그 호스트에 계정이 남지 않음) */
  gitSettingsGh?: { host: string; gh?: NonNullable<GitSettingsHost['gh']> };
  /** git-settings-check 결과 */
  gitSettingsCheck?: GitSettingsCheck;
  /** git-settings-install 결과 */
  gitSettingsInstall?: GitSettingsInstall;
  /** git-settings-gh-link 결과: 연결·되돌리기 뒤 그 호스트의 자격 상태 */
  gitSettingsGhLink?: { host: string; credential: NonNullable<GitSettingsHost['credential']> };
  /** git-settings-ssh-key 결과 */
  gitSettingsSshKey?: GitSettingsSshKey;
  issues?: GithubIssuesResult;
  issue?: GithubIssueResult;
  issueLink?: { link: GithubIssueLink };
  extIssues?: ExtIssuesResult;
  extIssue?: ExtIssueResult;
  extIssueLink?: { link: ExtIssueLink };
  checkLog?: GithubCheckLogResult;
  reviews?: GithubReviewsResult;
  prDraft?: GithubPrDraft;
  prCreate?: GithubPrCreateResult;
  history?: HistoryList;
  commit?: CommitDetail;
  commitFile?: BaseFileView & { oldPath?: string; incomplete?: string };
  fileHistory?: FileHistory;
  versions?: FileVersionList;
  version?: FileVersionView;
  /** read-file-version-image 결과 */
  versionImage?: { image: VersionImageSide; baseImage?: VersionImageSide };
  /** version-attach 결과: 첨부한 사본 */
  attached?: { sessionId: string; attachmentId: string; name: string; size: number };
  list?: ChangesList;
  /** check-ignored 결과: 보낸 경로 중 무시 대상인 것(보낸 글 그대로) */
  ignored?: string[];
  view?: ChangeView;
  /** 쓰기 결과. conflict면 쓰지 않았고 view가 지금 내용이다. */
  saved?: { conflict: boolean; indexVersion?: string; fileVersion?: string;
    /** save-generated-image 결과: 작업 폴더에 저장한 그림(conflict 는 false) */
    path?: string; name?: string };
  /** 휴지통으로 옮길 추적 안 된 새 파일(주 프로세스가 옮긴다). 검사한 대상의 승인 */
  trash?: ProjectTrashApproval;
  abs?: string;
  committed?: { hash: string; summary: string };
  pushed?: { remote: string; branch: string; upstreamSet: boolean };
  /** 커밋·푸시가 거절되거나 실패했을 때: 사람이 읽을 까닭, Git 출력 끝부분, 터미널에서 다시 할 명령 */
  failed?: { reason: string; reasonI18n?: WireText; output: string; terminalCommand?: string };
  remote?: GitRemoteStatus;
  badge?: GitBadge;
  /** stage-files 결과: 받은 경로와 판이 달라 받지 않은 경로(연 폴더 기준) */
  stagedFiles?: { staged: string[]; stale: string[] };
  fetched?: { remotes: string[]; at: number };
  pulled?: { from: string; count: number; head: string };
  /** fetch·pull·target 푸시 실패: 분류와 Git 출력 끝부분(최대 2,000자, 사용자 정보 지운 꼴) */
  gitFailure?: { code: GitFailureCode; output: string };
}
export type { ProjectFileEntry, ProjectSearchHit, ProjectTrashApproval, ProjectReplaceTarget, ProjectReplacePreviewFile, ProjectTextView, ProjectFilesAction, ProjectScript, ProjectScripts, ProjectFilesResult } from './project-files-types';
/** 외부 편집기(docs/notes/projects-slice2-2026-09-27.md 8번). id가 default면 기본 앱, reveal이면 Finder·탐색기에서 보기다. */
export interface ExternalEditor { id: string; name: string; line: boolean }

/**
 * 다음 요청이 지금 엔진에 붙일 인계 글이 한도로 앞 요청을 빼는데 그 요청을 다 덮는 요약이 없을 때, 요약을 만들어 넘기는 길
 * (docs/notes/summary-handoff-2026-09-26.md 1·2번). 요약할 엔진이 없거나 실행 중이면 없다.
 */
export interface HandoffPlan {
  /** 다음 요청을 받을 엔진 */
  to: EngineId;
  /** 인계 글이 한도로 뺄 앞 요청 수와 그중 이미 있는 요약이 덮는 수 */
  omitted: number;
  covered: number;
  /** 요약할 엔진과 방식(원래 엔진 대화의 갈래나 앱이 보관한 기록) */
  by: EngineId;
  source: 'thread' | 'record';
}

/** 대화 상태가 싣는 기본 최근 요청 수이자 칸이 처음 그리는 수(docs/notes/long-conversation-2026-09-24.md 1·5번). */
export const TURN_WINDOW = 30;

/**
 * 프로젝트 참여자 범위(docs/notes/project-invites-2026-10-04.md N1·N14). 주 프로세스 관문만 실행부에 묻는다(원격 요청 kind 가 아니다).
 * projects 는 등록부 소속 중 실행부에 등록돼 있고 다른 프로젝트와 실제 경로가 겹치지 않는 것만, sessions 는 그 프로젝트 소속 대화(임시 대화 빼고).
 */
export interface ParticipantScope {
  projects: Array<{ id: string; name: string; path: string; realPath: string }>;
  /** 소속은 있지만 폴더가 겹치거나 지워져 보이지 않는 프로젝트 번호 */
  hidden: string[];
  sessionIds: string[];
  /** sessionIds 중 참여자 대화(runAs)의 실행 주체 사람 번호(docs/notes/participant-runs-2026-10-04.md R4). 없는 대화는 주인 대화다. */
  runAs?: Record<string, string>;
}
/**
 * 참여자 실행(docs/notes/participant-runs-2026-10-04.md R1~R7, 논쟁 반영 M1~M15). 엔진은 Claude·Codex 만, 계정은 참여자 자기 API 키만이다.
 * 공급자는 엔진마다 하나로 고정한다(다른 계정으로 넘어가지 않는다).
 */
export type ParticipantRunEngine = 'claude' | 'codex';
export type { ParticipantKeyProvider } from './turn-execution';
export const PARTICIPANT_RUN_ENGINES: readonly ParticipantRunEngine[] = ['claude', 'codex'];
export const PARTICIPANT_KEY_PROVIDER: Record<ParticipantRunEngine, ParticipantKeyProvider> = { claude: 'anthropic', codex: 'openai' };
/** 이 컴퓨터의 참여자 실행 경계. 실행부가 처음 필요할 때 스스로 시험해 정하고 앱이 도는 동안 기억한다(R1). available 이 아니면 reason 이 화면 글(합쇼체)이다. */
export interface ParticipantRunBoundary { available: boolean; kind?: 'seatbelt'; reason?: string; reasonI18n?: WireText }
/**
 * 주 프로세스가 실행부로 보내는 참여자 실행 허가(늘 전체 묶음, 메모리에만, provider-keys 와 같은 통로). 등록부의 실행 허용(ParticipantAccess.run)과
 * 참여자 키 보관소에서 만든다. 실행부는 엔진을 띄우기 직전·재시도마다 이 묶음으로 다시 판정하고(M8), 묶음에서 빠진 사람·프로젝트의 실행은 멈춘다.
 * forget 은 빠진 사람 번호로, 실행부가 실행을 멈춘 뒤 그 사람의 참여자 홈을 지운다(M2·M9).
 */
export interface ParticipantRunGrant { personId: string; name: string; generation: number; projectIds: string[]; keys: Partial<Record<ParticipantKeyProvider, string>> }
/**
 * 조직 정책의 실행 한도(docs/notes/org-controls-2026-10-04.md G7). 허가 묶음 서명에 들어간다. 실행부는 엔진을 띄우기 직전·재시도마다 본다.
 * allowed 가 꺼지거나 engines 에서 빠진 엔진의 도는 실행·대기 줄은 M8 길로 멈춘다. perPerson·total 은 참여자 실행 슬롯(준비·실행·중지 중 포함,
 * 재시도는 같은 슬롯) 수 상한이고 null 이면 제한 없음. 낮추면 새 실행만 막는다(run-limit).
 */
export interface ParticipantRunLimits { allowed: boolean; engines: ParticipantRunEngine[]; perPerson: number | null; total: number | null }
export interface ParticipantRunGrants { grants: ParticipantRunGrant[]; forget: string[]; forgetProjects: Array<{ personId: string; projectId: string }>; limits: ParticipantRunLimits }
/**
 * 주 프로세스가 실행부에 보내는 조직 정책 중 실행부가 지키는 부분(G6·G8·G9). 실행부는 이 메시지를 받기 전까지 share-create 를 막는다.
 * share-create 확정 때 지금 값으로 다시 본다. viewerComment 가 거짓이면 대상 프로젝트에서 보기자인 참여자의 댓글 쓰기를 policy 로 거절한다.
 * auditRetentionDays 는 실행부 감사 파일 지우기에 쓴다.
 */
export interface BackendOrgPolicy { shareMaxDays: 1 | 7 | 30; shareAllowRaw: boolean; shareRemoteDevices: boolean; viewerComment: boolean; auditRetentionDays: number }
/** 참여자 화면에 싣는 실행 정보. keys 는 끝 네 자리만이다(R2·R7). projectIds 는 이 사람이 실행할 수 있는 프로젝트(편집자·실행 허용·경계 있음). */
export interface ParticipantRunView { boundary: ParticipantRunBoundary; projectIds: string[]; keys: Partial<Record<ParticipantKeyProvider, string>> }
/** 실행부에 참여자 투영을 부탁할 때 관문이 덮어쓰는 칸(N6). 기기가 보낸 같은 이름의 칸은 관문이 지운다. personId·run 은 7-4 에서 더했다. */
export interface ParticipantProjection { projectIds: string[]; personId?: string; run?: ParticipantRunView }

/*
 * 단계 7-5 댓글과 공동 편집(docs/notes/comments-coedit-2026-10-04.md, 논쟁 반영 K1~K15 가 C1~C8 보다 앞선다).
 * 댓글은 메시지 위치·문서 판에 붙고 소유 자료를 바꾸는 권한과 따로 판정한다. 공동 편집은 이 컴퓨터 실행부가 문서마다 버퍼 하나를 원본으로 들고
 * 변경 순서를 정한다(CodeMirror 중앙 권위 거절 방식, 변경은 @codemirror/state ChangeSet.toJSON). 글 기준은 BOM 없는 LF 다(K8).
 */
export const COMMENT_BODY_MAX = 4000;
export const COMMENT_QUOTE_MAX = 500;
/** 인용 앞뒤 문맥 글자 수(K14·K15) */
export const COMMENT_CONTEXT_MAX = 32;
export const COMMENT_REPLIES_MAX = 200;
export const COMMENT_THREADS_PER_TARGET_MAX = 500;
/** 공동 버퍼가 들고 있는 최근 변경 기록 수(K5). 넘게 뒤처진 화면은 reset 을 받는다. */
export const COEDIT_HISTORY_MAX = 2000;
/** 존재 신호 주기와 참가자를 빼는 무소식 시간(C4·K6) */
export const COEDIT_PRESENCE_MS = 20_000;
export const COEDIT_PEER_TIMEOUT_MS = 45_000;
/** 참가자 색 수. 대표 오렌지·위험·경고·성공 의미색은 쓰지 않는다(C4). */
export const COEDIT_COLORS = 8;

/**
 * 요청 출처(K1). 주 프로세스 창구만 출처에서 만들어 실행부 요청에 싣는다. 기기가 보낸 값은 쓰지 않는다.
 * key 는 참가자 묶기 열쇠(K2): 이 컴퓨터 창은 'window', 원격은 기기 번호. access 는 그 사람의 프로젝트별 역할 전부다.
 */
export type CollabActor =
  | { kind: 'owner'; key: string; place: string; readOnly?: true }
  | { kind: 'participant'; key: string; personId: string; name: string; access: Array<{ projectId: string; role: 'editor' | 'viewer' }> };
/** 작성자. 참여자에게 가는 응답·사건에서는 주인의 place 를 뺀다(권고 2, 화면은 "소유자"). */
export type CommentAuthor = { kind: 'owner'; place?: string } | { kind: 'participant'; personId: string; name: string };
/** 메시지 앵커(K14): 보이는 글 기준 인용과 앞뒤 문맥. 없으면 메시지 전체 댓글이다. */
export interface MessageCommentAnchor { quote: string; prefix: string; suffix: string }
/**
 * 문서 앵커(K15). version 은 단 순간 보던 저장 판 해시(sha256, FileVersionLine.sha256 과 같은 값), 저장 전 버퍼에 달았으면 null.
 * bufferVersion 은 공동 버퍼 판(있을 때), from·to 는 그 글 기준 글자 위치, line 은 시작 줄(1부터).
 */
export interface DocumentCommentAnchor { version: string | null; bufferVersion?: number; from: number; to: number; quote: string; prefix: string; suffix: string; line: number }
export type CommentPart = 'prompt' | 'reply';
/** 새 묶음의 대상. 문서의 folder 는 프로젝트 경로(참여자는 가상 경로), path 는 프로젝트 기준 상대 경로('/' 구분). */
export type CommentTarget =
  | { kind: 'message'; sessionId: string; turnId: string; part: CommentPart }
  | { kind: 'document'; folder: string; path: string };
/** 목록 대상: 대화 하나의 모든 메시지 댓글 또는 파일 하나의 댓글 */
export type CommentListTarget = { kind: 'message'; sessionId: string } | { kind: 'document'; folder: string; path: string };
export interface CommentView {
  id: string;
  /** 지운 댓글은 본문 없이 deleted 만 남는다 */
  author: CommentAuthor;
  body: string;
  createdAt: string;
  editedAt?: string;
  deleted?: true;
  /** 이 요청자가 고치기·지우기를 할 수 있는가(C3 표, 실행부가 정한다) */
  canEdit: boolean;
  canDelete: boolean;
}
export interface CommentThreadView {
  id: string;
  /** 문서 대상은 projectId 와 실제 파일 기준 상대 경로(K12)만 싣는다(절대 경로 없음, K4). */
  target: { kind: 'message'; sessionId: string; turnId: string; part: CommentPart } | { kind: 'document'; projectId: string; path: string };
  anchor?: MessageCommentAnchor | DocumentCommentAnchor;
  createdAt: string;
  resolvedAt?: string;
  resolvedBy?: CommentAuthor;
  comments: CommentView[];
}
export type CommentsAction =
  | { kind: 'list'; target: CommentListTarget }
  /** 턴 번호마다 열린 묶음 수 */
  | { kind: 'counts'; sessionId: string }
  | { kind: 'create'; target: CommentTarget; anchor?: MessageCommentAnchor | DocumentCommentAnchor; body: string }
  | { kind: 'reply'; threadId: string; body: string }
  | { kind: 'edit'; commentId: string; body: string }
  | { kind: 'delete'; commentId: string }
  | { kind: 'resolve'; threadId: string }
  | { kind: 'reopen'; threadId: string };
export interface CommentsResult {
  threads?: CommentThreadView[];
  counts?: Record<string, number>;
  /** 바꾼 묶음(create·reply·edit·delete·resolve·reopen) */
  thread?: CommentThreadView;
  /** 이 요청자가 이 대상에 댓글을 달 수 있는가(보기만 기기는 false) */
  canComment: boolean;
}
/** 실행부 → 주 프로세스 → 창·원격 문(projectId 로 참여자 거름). 받은 화면은 그 대상이 열려 있으면 list 를 다시 부른다. */
export interface CommentsChangedEvent {
  target: { kind: 'message'; sessionId: string; turnId: string } | { kind: 'document'; projectId: string; path: string };
  projectId: string | null;
}

import type { CoeditAction, CoeditEvent, CoeditResult } from './coedit-types';
export type { CoeditRole, CoeditPeer, CoeditUpdate, CoeditAction, CoeditOpenResult, CoeditReply, CoeditResult, CoeditEvent, CoeditDelivery } from './coedit-types';

export interface Snapshot extends SnapshotDiffFields, TurnDiffFields, AnswerDetailFields, AppSettingSnapshot, ProjectGroupFields, AutoCompactFields {
  /** 대화 태그 목록(앱 전체 공용, 배열 순서가 메뉴 순서). name 이 없는 태그는 그 색의 기본 이름(i18n sidebar.convColor.<id>)을 쓴다. */
  tags: ConversationTag[];
  /** 참여자 투영이면 실린다(N6·N13). 화면은 이 값이 있으면 주인 설정·실행 칸을 쓰지 않는다. personId·run 은 참여자 실행(R7). */
  participant?: { projectIds: string[]; personId?: string; run?: ParticipantRunView };
  /** 공통 기억이 바뀔 때마다 오르는 값. 화면은 이 값이 바뀌면 후보 상태와 기억 패널을 다시 받는다. */
  commonMemoryStamp?: number;
  /** 공통 규칙·지시 파일·기억 파일이 앱을 거쳐 바뀔 때마다 오르는 값. 화면은 바뀌면 보이는 규칙 패널만 다시 읽는다(B-181). */
  rulesStamp?: number;
  draft: Draft | null;
  folders: SessionFolder[];
  /** 새 폴더에 실제 폴더를 만들 위치(docs/notes/folder-paths-2026-10-01.md 2번). custom이 false면 처음 값(사용자 폴더 아래 LairWork)이다. */
  folderRoot: { path: string; custom: boolean; /** 새 폴더 위치로 정한 적 있는 정본 경로들. 앱 작업 폴더·등록 프로젝트와 같은 자격으로 믿는다(docs/notes/field-check-2026-10-01.md 3번). */ history: string[] };
  /** 등록한 프로젝트(등록 순). 소속은 화면이 대화의 작업 폴더로 계산한다(docs/notes/projects-slice1-2026-09-27.md 4번). */
  projects: Project[];
  /** 프로젝트·대화 작업 폴더·앱 작업 폴더의 지금 상태. 경로가 열쇠다(같은 문서 6·7번). */
  folderStatus: Record<string, FolderStatus>;
  sessions: SessionSummary[];
  /** 작업트리 관리 기록(지운 것 제외 안 함, 최근 사용 순). 화면용 보기다(docs/notes/worktree-2026-10-02.md D2·D5). */
  worktrees: WorktreeView[];
  /** 보여 주는 대화. turns는 최근 요청부터 요청한 창 크기(기본 TURN_WINDOW)만큼이다. */
  session: Session | null;
  /** session.turns 앞에 뺀 요청: 번호(오래된 순)와 그 요청들에 첨부나 결과 파일이 있는지. 뺀 요청이 없으면 없다. */
  olderTurns?: { ids: string[]; files: boolean };
  /** 사람이 직접 맡긴 위임(origin 'direct')의 번호(오래된 순). 요청 목록에 짝이 없어도 화면이 session.delegations 에서 이 번호의 위임을 독립 카드로 그린다. */
  directDelegations?: string[];
  /** 보여 주는 대화 전체의 사용량 합계(docs/notes/usage-2026-09-24.md 5번) */
  usage?: SessionUsage;
  /** Codex 상태(첫 실행 안내가 쓴다). 엔진별 상태는 engines. */
  engine: EngineInfo;
  engines: Record<EngineId, EngineInfo>;
  /** 엔진 모두 확인·업데이트 상태. 없으면 이 기능을 모르는 옛 호스트라 화면이 일괄 단추를 숨긴다. */
  engineUpdates?: EngineUpdatesView;
  firstRun: FirstRunState;
  workspace: WorkspaceState;
  storage: { error: string; errorI18n?: WireText; pendingResults: number };
  /**
   * 설정의 버전과 갱신 칸(docs/notes/update-safety-2026-09-24.md 2번). upgradeBackups는 갱신 전 백업을 두는 폴더이고,
   * upgradeBackup은 그 안의 가장 최근 백업이다. 없으면 null이다.
   */
  about: { version: string; build: string; packaged: boolean; storeVersion: number; dataDir: string; upgradeBackups: string;
    upgradeBackup: { name: string; path: string; from: number; to: number } | null;
    /** 이 컴퓨터 앱의 갱신 상태 사본(docs/notes/app-update-2026-10-03.md 화면과 원격). 주 프로세스가 단계가 바뀔 때 보낸다. 서버판·옛 주 프로세스는 없다. */
    update?: UpdateStatus };
  sessionLockReason?: string;
  /** 보여 주는 대화가 자기 작업 폴더 문제로 잠겨 다른 폴더로 옮길 수 있다(docs/notes/move-folder-2026-09-26.md 1번). */ sessionRelocatable?: boolean;
  /** 보여 주는 대화의 다음 인계가 한도로 앞 요청을 빼는데 요약을 만들어 넘길 수 있다(docs/notes/summary-handoff-2026-09-26.md 1번). */ handoffPlan?: HandoffPlan;
  /** 보여 주는 대화의 지금 실행에 추가 지시를 끼워 넣을 수 있다(대기 항목의 "지금 보내기"). */
  steerable?: boolean;
  /** 보여 주는 대화의 새 요청·첨부 허용 상태(docs/notes/refactor-phase4-2026-10-16.md 4i). 옛 판 호스트·참여자 상태는 없다. */ sessionRun?: SessionRun;
  /** 보여 주는 대화 첨부의 읽기 상태(docs/notes/documents-2026-10-03.md D1). 저장하지 않는 표시 상태다. */
  documents?: SnapshotDocuments;
  /** 모든 대화의 확인할 일. 정렬돼 있다. */
  attention: AttentionItem[];
  /** 예약·반복 목록(docs/notes/schedules-2026-10-04.md). 만든 순. */
  schedules: ScheduleView[];
  /** 기록 동기화 상태(docs/notes/encrypted-sync-2026-10-04.md S7). 비밀 문구·열쇠는 없다. */
  sync: SyncView;
  /** 예약 시각을 해석하는 이 컴퓨터(백엔드)의 시간대 이름. 예: Asia/Seoul */
  scheduleZone: string;
  /**
   * 운영체제 알림을 켰는가. 켠 경우에만 운영체제 알림을 낸다(Dock 배지와 창 제목 표시는 설정과 무관).
   * problem은 켜는 순간 운영체제가 확인 알림을 거절한 이유로, 켜기 요청의 응답에만 주 프로세스가 싣는다.
   */
  /** host: 다른 컴퓨터 창이 자기 설정으로 덮을 때 연결한 컴퓨터 값을 남기는 칸. */
  notifications: { os: boolean; problem?: string; host?: { os: boolean; problem?: string } };
  /** 전체 허용을 처음 한 번 동의했는가(docs/notes/effort-approval-2026-10-01.md 5.2) */
  approvalConsented: boolean;
  /**
   * 새 대화의 기본 엔진과 그 모델, 엔진별 기본 모델(docs/notes/new-chat-default-2026-09-24.md).
   * 저장된 값이 없으면 Codex와 엔진 연결의 기본 모델이다.
   */
  /** saved: 사용자가 새 대화 기본 엔진을 저장한 적이 있다(계정 기본만 저장한 것은 아니다). 첫 실행이 처음 골라 둘 엔진을 정할 때 쓴다(docs/notes/first-run-engine-2026-09-24.md 2번). */
  newChatDefault: NewChatDefaultSnapshot;
  /**
   * 엔진 로그인(기본 계정)의 표시 이름(name)과 Lair 에서 제거한 표식(removed). 엔진에 칸이 없으면 별칭이 없고 제거하지 않은 것이다.
   * 별칭이 없으면 화면이 기본 문구를 쓴다(docs/notes/settings-accounts-2026-10-08.md 3번).
   */
  engineLogins?: Partial<Record<EngineId, EngineLoginSetting>>;
  /** 앱이 모든 요청에 덧붙이는 지시의 지금 판과 글자 수(docs/notes/rules-2026-09-24.md 3번). 글은 src/logic/app-instructions.ts.
   * language 는 새 대화가 받을 지시문 언어(이 컴퓨터 화면 언어에서 정함, 다국어-7 G1). 지시 보기는 이 언어로 글을 만든다. */
  appInstructions: { version: string; chars: number; language: 'ko' | 'en'; codex?: { version: string; chars: number } };
  /**
   * 설정 「일반」의 답 언어(docs/notes/answer-language-2026-10-02.md L1). setting 은 저장한 값(기본 screen), screen 은 운영체제 언어에서 정한 화면 언어,
   * current 는 "지금: <언어 이름>" 칸이 보일 언어다(고른 언어나 화면 언어, "내가 쓴 말 따라"면 가장 최근 요청의 적용 언어가 있으면 그것 없으면 화면 언어).
   */
  answerLanguage: { setting: AnswerLanguageSetting; screen: { code: string; name: string }; current: { code: string; name: string } };
  /** 같은 부모의 하위 대화가 동시에 도는 지금 상한(설정이 없으면 SUB_SESSION_LIMITS.running, docs/notes/sub-sessions-2-2026-10-19.md 4.2). */
  subSessionRunning: number;
  /** 요청 예산 1묶음(설정, 없으면 DEFAULT_BUDGET 숫자에 defaultUnlimited=기본 제한 없음). 크기 메뉴 숫자가 이 값에 곱해 만든다. */
  budgetUnit: BudgetUnit;
  /** 판별 패치노트 상태(데이터 폴더 범위). 옛 호스트 응답에는 없을 수 있다. */
  releaseNotes?: ReleaseNotesView;
  /** 설정 「대화」의 "멈췄을 때"(docs/notes/failover-impl-2026-10-05.md R9). */
  recovery: RecoverySettings;
  /** 로컬 모델 공급처와 마지막 확인 결과(docs/notes/acp-engines-2026-10-05.md 설계 5절). custom 은 사용자가 목록을 저장했는지다. */
  localProviders: { custom: boolean; providers: LocalProviderView[] };
  /** Claude 대화에 사용자가 Claude Code에 붙인 MCP도 준다. 끄면 앱이 주는 MCP만이다(docs/notes/engine-mcp-2026-09-26.md 2·3번). */
  claudeUserMcp: boolean;
  /** MCP 연결 목록 요약(docs/notes/mcp-registry-2026-10-04.md R7). 명령·주소·환경·헤더는 없다. */
  mcp: McpView;
  /** Linear·Jira 이슈 출처 요약(10b). 토큰 없음. */
  issueSources: IssueSourcesView;
  /** 도구(MCP 서버)가 묻는 것: auto 면 바로 허용하고 진행 기록에 남긴다. 저장한 값이 없으면 auto(docs/notes/tool-ask-auto-2026-10-03.md T1). */
  toolAsk: 'auto' | 'ask';
  /** AI 가 앱 기능을 도구로 쓰게 할지(docs/notes/app-actions-2026-10-13.md 화면·논쟁 9). 없으면 켜짐으로 읽는다. 끈 값(false)만 저장한다. */
  appActionsEnabled?: boolean;
  /** 편집기 코드 완성(docs/notes/code-completion-2026-10-04.md C1). 원격 응답에는 싣지 않는다(forRemote 가 뺀다). */
  codeCompletion?: CodeCompletionView;
  /** 위임 켬 여부와 대상(docs/notes/delegation-2026-09-24.md 3번). 새 대화 기본값과 따로 저장한다. */
  delegation: { enabled: boolean; engine: EngineId; model: string; /** 사용자가 수락한 명령 위임 동의 판. 없으면 명령 방식은 거절한다. */ commandConsentVersion?: number;
    /** 셸에서 쓰는 위임 스크립트 이름(사용자가 등록, B-51). 이 이름의 셸 호출을 막고 표지를 붙인다. 없으면 등록 없음. */
    shellScripts?: string[];
    /** 전역 위임 정책(docs/notes/role-policy-2026-10-05.md 설계 2절). 없으면 정책 없음. */
    policy?: RolePolicy;
    /** 저장된 전역 정책을 읽지 못했다. 이때 정책이 걸린 위임은 거절한다(논쟁 5번: 옛 전역 대상으로 조용히 돌지 않게). */
    policyInvalid?: true;
    /** 저장된 전역 정책 읽기 상태(invalid 는 policyInvalid). retired=은퇴 엔진 대상만 빠짐, unknown=모르는 칸·값을 버림. 원문은 화면에 보내지 않는다. */
    policyState?: 'retired' | 'unknown';
    /** 전역 정책 층의 판(불투명 문자열). 화면에만 싣고 저장하지 않는다. 저장 요청의 expectedRevision 으로 되돌려 보낸다. */
    policyRevision?: string };
}

/**
 * 칸 하나가 답 조각 알림 때 받는 대화 상태(docs/notes/multi-pane-streaming-2026-09-25.md 6번). 전체 상태의 대화 부분과 같은 규칙으로 싣는다.
 * 대화가 없어졌으면 session이 null이다.
 */
export type SessionState = Pick<Snapshot, 'session' | 'olderTurns' | 'directDelegations' | 'usage' | 'sessionLockReason' | 'sessionRelocatable' | 'handoffPlan' | 'steerable' | 'sessionRun' | 'documents'> & TurnDiffFields;

/** 호스트가 신뢰하는 번호와 이번 기동 세대. 클라이언트가 hostId를 정하지 않는다(docs/notes/web-server-host-2026-09-30.md). */
export interface TrustedHostContext { hostId: string; bootGeneration: number }


/** prepare 시점의 실제 effective target. secret 없음. */
export interface ReceiptTarget { engine: EngineId; model: string; account?: string }

/** prepare-send·request-receipt·list-prepared·list-prepared-all 응답. */
export interface RequestReceiptView {
  requestId: string;
  sessionId: string;
  kind: 'send';
  state: RequestReceiptState;
  /** list-prepared-all 에만. 그 대화의 지금 제목(docs/notes/prepared-requests-2026-10-13.md 3.1절). */
  title?: string;
  contentHash?: string;
  preparedAt?: string;
  acceptedAt?: string;
  dispatchedAt?: string;
  completedAt?: string;
  executionId?: string;
  /** prepared 조회·목록에만. secret 없음. */
  text?: string;
  attachmentIds?: string[];
  draftRevision?: number;
  target?: ReceiptTarget;
}

// ---- 문서 이해와 프로젝트 자료(docs/notes/documents-2026-10-03.md, G1) ----

// 문서 종류·읽기 상태·원문 위치 모양(DocKindView·DocStateName·DocLocView·DocStateView)은 src/shared/session-run.ts 로 옮겼다. 아래 import·re-export 로 그대로 쓴다.
/** 화면 상태에 싣는 읽기 상태. 보여 주는 대화의 모든 첨부(입력칸 대기·대기열·보낸 요청)가 열쇠이고, 읽지 않는 형식·아직 시작 안 한 첨부는 칸이 없다. */
export interface SnapshotDocuments { attachments: Record<string, DocStateView> }

// 프로젝트 자료 상태·보기 모양(ProjectSourceHealth·ProjectSourceView·ProjectSourceList)과 한도는 src/shared/project-sources.ts 로 옮겼다. 아래 import·re-export 로 그대로 쓴다.
import type { ProjectSourceList } from './project-sources';
export type { ProjectSourceHealth, ProjectSourceView, ProjectSourceList } from './project-sources';
export { PROJECT_SOURCE_LIMIT } from './project-sources';

// 근거 기록 타입·한도는 evidence-types.ts 로 옮겼다(docs/notes/b568-doc-notes-i18n-2026-11-02.md D9). 기존 import 자리를 위해 다시 내보낸다.
export type { EvidenceSource, EvidenceItem, EvidenceCite, EvidenceSkipped, WebSearchRecord, WebSourceRecord, WebCite, WebEvidence, TurnEvidence } from './evidence-types';
export { EVIDENCE_LIMITS, WEB_EVIDENCE_LIMITS } from './evidence-types';

/** 문서 보기·근거 열기가 가리키는 자료. 첨부·결과 파일·작업 폴더 파일은 기존 파일 대상, 프로젝트 자료는 번호로 가리킨다. */
export type DocTarget = FileTarget | { projectId: string; sourceId: string };

/** 문서 보기 방식(source-view document). docx 는 읽기 전용 미리보기 HTML, sheet 는 표 창(시트·행·열), text 는 글 목표 줄 둘레. */
export type DocViewSpec =
  | { kind: 'docx' }
  | { kind: 'sheet'; sheet?: string; row?: number; col?: number; rows?: number; cols?: number }
  | { kind: 'text'; line: number; before?: number; after?: number };

/** 문서 보기 응답. 원문 판이 expectSource 와 다르면 ok=false, reason='changed' 다. */
export type DocumentView =
  | { kind: 'docx'; ok: boolean; reason?: string; reasonI18n?: WireText; name: string; html: string; blocks: number }
  | { kind: 'sheet'; ok: boolean; reason?: string; reasonI18n?: WireText; name: string; sourceVersion?: string; sheets: Array<{ name: string; rows: number; cols: number }>; sheet: string;
      row: number; col: number; rowCount: number; colCount: number; totalRows: number; totalCols: number; cells: string[][]; cut: Array<[number, number]> }
  | { kind: 'text'; ok: boolean; reason?: string; reasonI18n?: WireText; name: string; sourceVersion?: string; totalLines: number; from: number; to: number; target: number; text: string; truncated: boolean; targetTruncated?: boolean };

/**
 * 근거 열기 응답(source-view evidence). match 는 지금 파일의 원문 판이 근거를 뽑은 판과 같다는 뜻이다.
 * 같으면 target·loc 로 원문을 열고, 다르거나(changed) 없으면(missing·removed·unavailable) 원문 대신 저장 발췌(excerpt)를 보인다.
 */
export interface EvidenceOpen {
  s: number;
  c: number;
  name: string;
  kind: DocKindView;
  origin: 'request' | 'project';
  loc: DocLocView;
  /** 다른 요청의 기록이 가리키는 번호면 그 요청 번호(답을 낸 요청과 다를 때) */
  definedIn: string;
  match: boolean;
  /** 이 근거를 뽑은 때의 원문 판(sha256). 원문을 열 때 expectSource 로 넘겨 그사이 바뀌었는지 실행부가 다시 확인한다 */
  sourceVersion?: string;
  /** match 가 거짓일 때의 사유 */
  reason?: 'changed' | 'missing' | 'removed' | 'unavailable';
  reasonText?: string;
  /** match 가 참일 때 원문을 열 대상 */
  target?: DocTarget;
  /** 저장해 둔 발췌(없을 수 있다) */
  excerpt?: string;
  /** 발췌가 조각 안 어디서 시작하는지(EvidenceCite.excerptAt) */
  excerptAt?: { offset: number; line?: number };
  /** 지금 파일을 그대로 열 수 있는 대상(판이 바뀐 경우의 "지금 파일 열기") */
  current?: DocTarget;
}

/** handle·typed bridge 응답. 조회는 Snapshot과 분리한다. */
export type { CodeCompletionFormat, CodeCompletionSource, CodeCompletionSetting, CodeCompletionView, CodeCompleteResult, ProviderModelsResult } from './requests/code-completion';

export type { ShareFindings, ShareDraftSection, ShareDraftFile, ShareSkipped, ShareDraft, ShareItem, ShareList } from './requests/shares';

export type AppRequestResult = SettingsSnapshot | SettingsApplyItem[] | TransferExportResult | TransferPreview | TransferApplyResult | { restored: number } | Snapshot | ResetCreditResult | FolderScopeInfo | RelocatePlan | EnginePathResult | ShareDraft | ShareItem | ShareList | DelegationApplyPreview | DelegationChangeResult | RequestReceiptView | RequestReceiptView[] | PreparedChangeResult | SessionImageItem[] | SubSessionChild[] | ReportToParentResult | StagedFile[] | MultiSendResult | ProviderModelsResult | CodeCompleteResult | ImportedFoldersInspection | ImportedFoldersMapResult | ChatFolderProjectsInspection | WorktreeStatus | WorktreePrepResult | BranchResult | WorktreeMergePreview | WorktreeCleanPreview | WorktreeMergeResult | CopyApplyPreview | CopyCleanPreview | CopyChangeResult | RestorePlan | RestoreRunResult | FolderStatus | ProjectSourceList | EvidenceOpen | DocumentView | McpServerConfig | McpImportCandidates | SkillsView | SkillBody | SkillTranslatePlan | SkillTranslateResult | LairCustomSkillsView | SkillOff[] | McpLoginStart | McpCallList | { precheck?: SchedulePrecheck } | ScheduleListResult | ScheduleFinishResult | SyncProbeResult | BrowserOriginsResult | CommandAllowList | CliOverview | EngineImportSourcesResult | EngineImportListResult | EngineImportOpenResult | MigrateResult | { port: number } | null | { dataUrl: string } | { text: string; blocks: number; missing?: true } | McpMentionableResult | ReplySummaryResult | PaneSetResult;

export type AppRequest =
  | TransferRequest
  | SettingsRequest
  /** turnLimit: 보여 줄 대화에서 실을 최근 요청 수. 없으면 TURN_WINDOW. */
  | { kind: 'snapshot'; sessionId?: string; turnLimit?: number; known?: SnapshotKnown; recordScope?: RecordScopeRequest }
  | { kind: 'set-workspace'; path: string }
  | { kind: 'retry-save' }
  | EngineRequest
  /** account가 있으면 그 추가 계정의 한도를 읽는다(docs/notes/accounts-2026-09-26.md 12번). */
  | { kind: 'check-limits'; engine: EngineId; account?: string }
  /**
   * 앱 전용 폴더 로그인(docs/notes/grok-engine-2026-09-24.md 설계 6번). start 때는 grok이 기본 브라우저를 스스로 열고,
   * open은 응답의 login.url을 주 프로세스가 기본 브라우저로 다시 연다. dismiss는 실패한 로그인 칸을 닫는다.
   */
  | { kind: 'engine-login'; engine: EngineId; action: 'start' | 'open' | 'submit' | 'cancel' | 'dismiss' | 'logout'; code?: string }
  /**
   * 추가 계정 만들기·로그인·해제·삭제(docs/notes/accounts-2026-09-26.md 3·6·7번). submit은 브라우저에서 받은 코드를 넘기고,
   * open은 응답의 로그인 주소를 주 프로세스가 기본 브라우저로 다시 연다.
   */
  | { kind: 'account-create'; engine: EngineId; name: string; provider?: ProviderConfig }
  /** 연결 계정의 모델 목록·키 필요 여부 고치기, 주소에서 모델 목록 불러오기(docs/notes/provider-accounts-2026-10-03.md 논쟁 6·8). */
  | { kind: 'account-provider-update'; accountId: string; models?: string[]; needsKey?: boolean }
  | { kind: 'account-provider-models'; accountId: string }
  /** 편집기 코드 완성 설정 저장·완성 요청·직접 주소 모델 불러오기. 이 컴퓨터의 Lair 앱 화면만(docs/notes/code-completion-2026-10-04.md C1·C2·C6). */
  | { kind: 'set-code-completion'; setting: CodeCompletionSetting }
  | { kind: 'code-complete'; name: string; prefix: string; suffix: string; generation: number }
  | { kind: 'code-complete-models'; baseUrl: string }
  | { kind: 'account-login'; accountId: string; action: 'start' | 'open' | 'submit' | 'cancel' | 'dismiss' | 'logout'; code?: string }
  /** replacement: 지우려는 계정이 지금 새 대화 기본이면 필수('default' 엔진 로그인 | 계정 번호 | null 기본 없음). 아니면 무시한다(docs/notes/settings-accounts-2026-10-08.md 3번 S4). */
  | { kind: 'account-delete'; accountId: string; replacement?: 'default' | string | null }
  /** 새 대화 기본 계정만 바꾼다. 기본 엔진·모델·강도·승인은 그대로다. 'default' 는 엔진 로그인(제거 상태면 거절), null 은 기본 없음(같은 문서 S1). */
  | { kind: 'set-default-account'; engine: EngineId; account: 'default' | string | null }
  | AccountCreditRequest
  /** 엔진 로그인을 Lair 에서 제거한다. 지금 새 대화 기본이면 replacement(계정 번호 | null 기본 없음) 필수. 인증·사용자 HOME 은 건드리지 않는다(같은 문서 S3). */
  | { kind: 'engine-login-remove'; engine: EngineId; replacement?: string | null }
  /** 제거한 엔진 로그인을 다시 등록한다. 새 대화 기본은 바꾸지 않는다(같은 문서 S3). */
  | { kind: 'engine-login-restore'; engine: EngineId }
  /** engine: 1단계에서 고른 엔진. 넘어갈 때 새 대화 기본 엔진으로 저장한다(docs/notes/first-run-engine-2026-09-24.md 3번). */
  | { kind: 'first-run'; action: 'go'; step: 1 | 2 | 3; engine?: EngineId }
  | { kind: 'first-run'; action: 'trust'; path: string }
  | { kind: 'first-run'; action: 'finish' }
  /** 확인 창에서 사람이 고른 건너뛰기. 준비와 상관없이 마친다(docs/notes/first-run-fork-2026-09-29.md 3절). */
  | { kind: 'first-run'; action: 'skip'; /** 시작 안내의 '나중에'. 확인 창 없이 닫고, 준비 전에는 보내기를 막는다(docs/notes/onboarding-board-2026-10-09.md). */ later?: true }
  /** 첫 화면에서 고른 길을 저장한다. null이면 아직 고르지 않음으로 되돌린다(2절). */
  | { kind: 'first-run'; action: 'route'; route: 'local' | 'remote' | null }
  /** terminal이면 엔진 터미널 대화다(docs/notes/terminal-slice2-2026-09-27.md 1번). */
  | { kind: 'create-session'; projectPath: string; engine?: EngineId; model: string; effort?: string | null; /** 폰 요청만 싣는다. 지원하지 않는 강도를 조용히 엔진 기본으로 바꾸지 않고 거절한다. */ effortStrict?: true; folderId?: string;
    /** 추가 계정 번호. 'default' 는 엔진 로그인 명시이고 누락과 같은 뜻으로 대화에 저장하지 않는다. null 은 받지 않는다(docs/notes/settings-accounts-2026-10-08.md 4절 F2). */
    account?: 'default' | string; terminal?: boolean; mode?: RequestMode;
    /** 새 대화 칸에서 고른 속도 등급 id. 고른 모델(계정)의 speeds 에 없으면 거절한다. null·없음은 엔진 기본이다(docs/notes/engine-models-2026-10-28.md D5). */
    speed?: string | null;
    /** 관문만 싣는다(참여자 대화 만들기, docs/notes/participant-runs-2026-10-04.md R4). 있으면 실행부가 runAs 를 이 사람으로 정한다. */
    participant?: ParticipantProjection;
    /** 작업 위치: 앱 작업 루트에서 연 대화를 대화 폴더로 돌릴지('chat-folder', 없으면 기본) 루트 그대로('here') 둘지(N5). */
    place?: 'chat-folder' | 'here';
    /** 새 대화 칸에서 미리 고른 웹 검색 설정. 첫 요청부터 적용한다(docs/notes/web-sources-2026-10-03.md W1). 없으면 기본. */
    web?: SessionWebPolicy;
    /** 새 대화 칸에서 미리 고른 공통 기억 끔. 첫 요청부터 적용한다(docs/notes/common-memory-2026-10-03.md M5). */
    memoryOff?: boolean;
    /** 임시 대화로 만든다. 메모리에만 두고 저장하지 않는다(docs/notes/temporary-chat-2026-10-03.md T4). */
    temporary?: true;
    /** 이 프로젝트의 새 대화로 만든다. 없으면 소속 없음. 정리 폴더(folderId)가 없으면 그 프로젝트의 기본 정리 폴더에 넣는다(docs/notes/sidebar-scope-2026-10-02.md S4·논쟁 3). */
    projectId?: string;
    /** 첫 실행 요청을 보낼 때 projectPath 의 저장소에서 새 작업트리를 만든다(docs/notes/worktree-2026-10-02.md D2). base 는 기준 브랜치 이름. */
    isolation?: { kind: 'worktree'; base?: string } | { kind: 'copy' };
    /** 부모 대화 번호. AI 는 자기 대화 번호만 줄 수 있다(docs/notes/sub-sessions-2026-10-19.md 4.7·4.8절). */
    parentSessionId?: string;
    /** 문이 정한다. AI 앱 동작으로 온 요청이면 'ai', 아니면 없다(사람). */
    parentBy?: 'ai' }
  | WorktreeReadRequest
  /**
   * 폴더 하나의 지금 상태를 읽는다(보기 요청, docs/notes/safe-work-leftovers-2026-10-03.md S6·K9). 응답은 FolderStatus, 범위 밖이거나 확인하지 못하면 null.
   * 감시를 새로 걸지 않고 스냅숏의 폴더 상태 목록도 건드리지 않는다.
   */
  | { kind: 'folder-status'; path: string }
  /** 작업트리 준비를 읽는다(docs/notes/worktree-2026-10-02.md D3, 보기 요청). settings·preview 는 sourceRoot, output 은 worktreeId, env 는 cwd. */
  | { kind: 'worktree-prep-view'; action: 'settings' | 'preview'; sourceRoot: string }
  | { kind: 'worktree-prep-view'; action: 'output'; worktreeId: string }
  | { kind: 'worktree-prep-view'; action: 'env'; cwd: string }
  /** 작업트리 준비를 바꾼다(같은 문서 D3, 이 컴퓨터에서만). 준비 명령 저장·복사 목록 승인(미리보기의 hash)·준비 다시 실행·건너뛰고 시작. */
  | { kind: 'worktree-prep'; action: 'save-command'; sourceRoot: string; command: string }
  | { kind: 'worktree-prep'; action: 'approve'; sourceRoot: string; hash: string }
  | { kind: 'worktree-prep'; action: 'rerun' | 'skip'; worktreeId: string }
  /**
   * 브랜치 보기(docs/notes/worktree-2026-10-02.md D7·D4, 보기 요청). list 는 folder 의 저장소 로컬 브랜치,
   * base-files·base-file 은 작업트리 대화의 병합 기준점 대비 바뀐 파일 목록과 파일 하나의 앞뒤다.
   */
  | { kind: 'branch-view'; action: 'list'; folder: string }
  | { kind: 'branch-view'; action: 'base-files'; worktreeId: string }
  | { kind: 'branch-view'; action: 'base-file'; worktreeId: string; path: string }
  /**
   * 다른 모델 실행 비교 갈래의 바뀐 파일 보기(감사 K12, docs/notes/rewind-branch-2026-10-02.md D5). compare-files 는 갈래 대화(sessionId)의 비교 기준(그 요청 앞 상태)과 선택한 답의 뒤 상태(없으면 지금 작업트리)
   * 사이에 바뀐 파일 목록, compare-file 은 그 목록이 알려 준 뒤 스냅숏(after)에서 파일 하나의 앞뒤다. 응답은 base-files·base-file 과 같은 칸(baseChanges·baseFile)이다.
   */
  /** turnId: 결과 카드에서 연 비교는 그 답의 요청 번호를 넘겨 그 답의 뒤 상태로 고정한다(감사 고침 확인 R4). 없으면 마지막 실제 요청. */
  | { kind: 'branch-view'; action: 'compare-files'; sessionId: string; turnId?: string }
  | { kind: 'branch-view'; action: 'compare-file'; sessionId: string; path: string; after: string }
  /**
   * 쓰기 위임 후보의 바뀐 파일 보기(docs/notes/write-subtasks-2026-10-04.md W5). 후보 시작 ↔ 후보 결과 스냅숏 사이. 응답은 base-files·base-file 과 같은 칸(baseChanges·baseFile)이다.
   * 사본을 지운 뒤에도 스냅숏으로 보인다.
   */
  | { kind: 'branch-view'; action: 'delegation-files'; sessionId: string; delegationId: string; attemptId: string }
  | { kind: 'branch-view'; action: 'delegation-file'; sessionId: string; delegationId: string; attemptId: string; path: string }
  /** 합치기·정리 미리보기(docs/notes/worktree-2026-10-02.md D4·D6, 보기 요청). merge 는 WorktreeMergePreview, clean 은 WorktreeCleanPreview. */
  | { kind: 'worktree-merge-view'; action: 'merge'; worktreeId: string }
  | { kind: 'worktree-merge-view'; action: 'clean'; worktreeId: string }
  /**
   * 합치기·충돌 뒤 기준 받기·정리·복구 다시 확인(같은 문서 D4·D6, 원격 웹도 같은 검사: docs/notes/worktree-2026-10-02.md D9·198줄). merge 의 expectHead 는 미리보기의 head 다.
   * clean 은 잃을 것이 있으면 discard 가 true 일 때만, deleteBranch 는 자동 삭제가 안 되는 브랜치를 사용자가 고른 때만 쓴다.
   */
  | { kind: 'worktree-merge'; action: 'merge'; worktreeId: string; method: WorktreeMergeMethod; message: string; expectHead: string }
  | { kind: 'worktree-merge'; action: 'sync-base' | 'recheck'; worktreeId: string }
  | { kind: 'worktree-merge'; action: 'clean'; worktreeId: string; discard: boolean; deleteBranch: boolean }
  /** 격리 복사본 반영·정리 미리보기(docs/notes/worktree-2026-10-02.md D8, 보기 요청). apply 는 CopyApplyPreview, clean 은 CopyCleanPreview. */
  | { kind: 'copy-view'; action: 'apply'; worktreeId: string }
  | { kind: 'copy-view'; action: 'clean'; worktreeId: string }
  /** 격리 복사본 반영(expect 는 미리보기의 version)·정리(잃을 것이 있으면 discard 가 true 일 때만). 원격 웹도 같은 검사(docs/notes/worktree-2026-10-02.md D9·198줄)(D8). */
  | { kind: 'copy-change'; action: 'apply'; worktreeId: string; expect: string }
  | { kind: 'copy-change'; action: 'clean'; worktreeId: string; discard: boolean }
  /**
   * 파일 되돌리기 계획(docs/notes/rewind-branch-2026-10-02.md D2, 보기 요청). turnId 는 그 요청 앞으로, opId 는 그 되돌리기 작업의 취소다.
   * applyTurnId(sessionId 는 다른 모델 갈래 대화, 이 갈래 답의 요청 번호)는 그 답을 원래 폴더에 적용하는 계획이다(D5 [논쟁 8]). 셋 중 하나만 쓴다.
   * 판정용 임시 스냅숏을 남긴다.
   */
  | { kind: 'restore-plan'; sessionId: string; turnId?: string; opId?: string; applyTurnId?: string }
  /** 파일 되돌리기 실행(같은 문서 D2). planHash 는 계획의 hash, overwrite 는 보호 중 덮어쓰기로 고른 경로다. 같은 requestId 는 같은 작업 결과를 돌려준다.
   *  apply 가 true 면 turnId 는 갈래 대화(sessionId)의 답 요청 번호이고 그 답을 원래 폴더에 적용한다(D5). */
  | { kind: 'restore-run'; sessionId: string; turnId: string; apply?: true; requestId: string; planHash: string; overwrite: string[] }
  /** 되돌리기 취소(같은 문서 D2). opId 는 취소할 되돌리기 작업이다. */
  | { kind: 'restore-undo'; sessionId: string; opId: string; requestId: string; planHash: string; overwrite: string[] }
  /** 화면이 휴지통으로 옮긴 결과를 알린다. 알려야 되돌리기 작업이 끝난다(휴지통은 주 프로세스만 쓴다). */
  /** unknown: 이동 요청이 끝났는지 알 수 없다(호출 실패·응답 없음). 이동부가 명시적으로 거절했을 때만 trashed false 에 unknown 이 없다(감사 고침 확인 R2). */
  | { kind: 'restore-trashed'; opId: string; results: Array<{ path: string; trashed: boolean; unknown?: true; reason?: string }> }
  /**
   * 갈래 만들기(docs/notes/rewind-branch-2026-10-02.md D3). atTurnId 요청 앞의 요청들을 복사한 새 대화를 만든다(파일은 건드리지 않는다).
   * 같은 requestId 는 같은 대화를 돌려준다. startOp 는 둘 다일 때 되돌리기 작업 번호. engine·model·effort 는 compare 용 자리(없으면 원래 대화 값).
   * input 은 갈래 첫 요청 입력 자리다: text 가 있으면 새 대화 초안에 채우고, attachments 가 참이면 갈라진 요청의 첨부를 새 대화 소유로 복사해
   * 대기 첨부에 둔다. 되돌리기(rewind)는 둘 다 기본으로 채운다. 보내지는 않는다. input.send 가 있으면(edit·regenerate) 입력칸은 채우지 않고
   * 갈라진 요청의 입력을 기록에서 재현해 첫 요청으로 실어 보낸다(D4). 같은 requestId 는 갈래도 첫 요청도 한 번만 만든다.
   * whole 이 참이면(branchKind 는 rewind, input·startOp 없이) atTurnId 요청까지 포함해 복사하고 입력칸은 채우지 않는다(대화 복사). 도는 중인 요청이면 거절한다.
   */
  | { kind: 'branch-create'; sessionId: string; atTurnId: string; branchKind: BranchKind; requestId: string; startOp?: string;
      engine?: EngineId; model?: string; effort?: string | null; input?: BranchInput; whole?: boolean }
  /** 브랜치 만들기(지금 커밋에서, 커밋 안 한 변경은 그대로)·전환·이름 바꾸기(from 이 없으면 지금 브랜치). 같은 문서 D7. */
  | { kind: 'branch-change'; action: 'create' | 'switch'; folder: string; name: string }
  | { kind: 'branch-change'; action: 'rename'; folder: string; name: string; from?: string }
  /**
   * 만든 대화의 엔진·모델·계정을 바꾼다. 다음 요청부터 적용한다(docs/notes/engine-switch-2026-09-24.md 1번). account가 null이면 기본 계정,
   * 없으면 그대로다(docs/notes/accounts-2026-09-26.md 9번).
   */
  | { kind: 'set-session-model'; sessionId: string; model: string; engine?: EngineId; account?: string | null; effort?: string | null }
  /** 다음 요청부터 적용한다. null이면 엔진 기본이다(docs/notes/effort-approval-2026-10-01.md 1.2). */
  | { kind: 'set-session-effort'; sessionId: string; effort: string | null }
  /** 다음 요청부터 적용한다. null이면 엔진 기본이다. 값은 지금 모델(고른 계정)이 알린 속도 등급 id 만 받는다(docs/notes/engine-models-2026-10-28.md D5). */
  | { kind: 'set-session-speed'; sessionId: string; speed: string | null }
  /** 다음 요청의 모드(docs/notes/plan-mode-2026-10-02.md D1). 도는 요청은 시작 때 모드를 유지한다. */
  | { kind: 'set-session-mode'; sessionId: string; mode: RequestMode }
  /** 대기 계획을 승인해 실행 요청을 접수한다(같은 문서 D8). 같은 계획이면 같은 실행 요청을 돌려준다. */
  | { kind: 'approve-plan'; sessionId: string; planTurnId: string }
  | { kind: 'dismiss-plan'; sessionId: string; planTurnId: string }
  | ProjectPlaceRequest
  /** Codex 터미널의 화면 신호(docs/notes/terminal-slice2-2026-09-27.md 4번): 입력(Enter), 턴 끝·승인 요청(OSC 9). */
  | { kind: 'terminal-signal'; sessionId: string; signal: 'input' | 'done' | 'asking' }
  /** 작업 폴더 문제로 잠긴 대화를 고른 폴더로 옮긴다(docs/notes/move-folder-2026-09-26.md). 주 프로세스는 선택창으로 고른 폴더만 넘긴다. */
  | { kind: 'relocate-session'; sessionId: string; projectPath: string; expectFrom?: string; projectId?: string | null; undoOf?: string }
  | { kind: 'rename-session'; sessionId: string; title: string }
  | { kind: 'set-session-pinned'; sessionId: string; pinned: boolean }
  /**
   * 보내기. preparedRequestId가 있으면 그 접수 번호의 prepared 내용만 실행한다(웹 재연결 경로).
   * 로컬·앱은 preparedRequestId 없이 기존과 같다.
   */
  | { kind: 'send'; sessionId: string; requestId: string; text: string; attachmentIds?: string[]; draftRevision?: number; preparedRequestId?: string;
    /** 관문만 싣는다(참여자 보내기, R4·R6). 있으면 실행부가 그 대화의 runAs 가 이 사람인지 본다. */
    participant?: ParticipantProjection;
    /** 아직 요청이 하나도 없는 새 대화의 첫 요청 모드(docs/notes/plan-mode-2026-10-02.md D1). 요청이 있는 대화에서는 무시한다. */
    mode?: RequestMode;
    /** 고쳐 달라는 계획을 낸 계획 요청 번호(같은 문서 D7). 있으면 도는 요청에 끼워 넣지 않고, 그 계획을 엔진 입력에 함께 싣는다. */
    planRevision?: string;
    /** 사람 입력창의 이번 요청 예산 크기(docs/notes/request-budget-knobs-2026-10-24.md 6절). AI 앱 동작에서는 떼어 낸다. */
    budgetSize?: BudgetSize;
    /** 프로젝트에 묶은 추가 폴더 가운데 없어진 것을 이번 요청에서 빼고 보낸다는 사용자 확인(docs/notes/multi-folder-2026-10-28.md 4.2). 사람 입력창만 싣는다. */
    omitMissingFolders?: true }
  | SessionRunRequest
  | AttachmentClipRequest
  /** 이 요청의 미리보기 브라우저 도구를 멈춘다(docs/notes/preview-browser-2026-10-02.md D5 멈추기). 요청 자체는 계속된다. */
  | { kind: 'browser-stop'; sessionId: string; turnId: string }
  /** 브라우저 바깥 주소 허용 목록 보기·지우기·모든 사이트 켜기/끄기(docs/notes/mid-features-2-2026-10-17.md B-166). 답은 BrowserOriginsResult. */
  | BrowserOriginsRequest
  /** 이 대화에서 계속 허용한 명령 목록 읽기·빼기(docs/notes/memo-board-b1-2026-10-21.md D). 답은 command-allows-list 가 CommandAllowList, command-allow-remove 는 화면 상태. */
  | CommandAllowRequest
  | SessionApprovalRequest
  /** 멈춘 요청 예산을 늘려서 계속하거나 끝낸다. 사람만(앱 동작 통로 닫힘). */
  | BudgetRequest
  | ReleaseNotesRequest
  /** 브라우저 근거 캡처 읽기(TurnBrowser). thumb 이면 320px 축소본. 응답은 { dataUrl } */
  | { kind: 'browser-capture'; sessionId: string; turnId: string; captureId: string; thumb?: boolean }
  /** 크게 보기용 대화 이미지 목록(보기 요청). 응답은 SessionImageItem[]. 대화의 모든 요청을 한 번 훑는다(docs/notes/image-viewer-2026-10-13.md 4.7절 1번). */
  | { kind: 'session-images'; sessionId: string }
  | RespondApprovalRequest
  | { kind: 'queue-edit'; sessionId: string; itemId: string; text: string }
  | { kind: 'queue-cancel'; sessionId: string; itemId: string }
  | { kind: 'queue-send'; sessionId: string; itemId: string }
  | { kind: 'queue-resume'; sessionId: string }
  | { kind: 'queue-clear'; sessionId: string }
  | { kind: 'mark-seen'; sessionId: string; through: string }
  | { kind: 'mark-unread'; sessionId: string }
  /** 대화 삭제(docs/notes/session-delete-2026-09-25.md). 복사해 둔 첨부는 남긴다. */
  | { kind: 'delete-session'; sessionId: string }
  /** 임시 대화 끝내기(docs/notes/temporary-chat-2026-10-03.md T7): 새 접수 막기, 승인·질문 취소, 실행·첨부 준비 확인, 임시 보관소 폴더 지우기, 대화·장부 버리기. stop 이 없으면 돌고 있는 요청이 있을 때 거절하고, stop 이면 멈추고 끝낸다. 답은 화면 상태. */
  /**
   * 문서 이해(G1)의 보기 요청(docs/notes/documents-2026-10-03.md D6·D7·D8, 원격도 보기). list 는 프로젝트 자료 목록(ProjectSourceList)이고 열 때 바뀜 후보(크기·수정 시각)를
   * 뒤에서 다시 읽는다. evidence 는 근거 열기(EvidenceOpen): 요청(turnId)이 낸 답의 근거 번호 s-c 를 실행부가 대상 파일·위치·원문 판 일치로 푼다.
   * document 는 문서 보기(DocumentView): 대상(첨부·결과·작업 폴더 파일 또는 프로젝트 자료)을 실행부가 기존 파일 경계·실제 경로·링크 검사로 푼다.
   * expectSource 는 근거의 원문 판(sourceVersion)이고, 다르면 ok=false·reason='changed' 로 알린다.
   */
  | { kind: 'source-view'; action: 'list'; projectId: string }
  /** excerpt: true 면 원문이 그대로(match)일 때도 저장 발췌를 붙인다(원문을 열지 못하는 폰 요청이 보낸다, 데스크톱은 보내지 않는다). */
  | { kind: 'source-view'; action: 'evidence'; sessionId: string; turnId: string; s: number; c: number; excerpt?: boolean }
  | { kind: 'source-view'; action: 'document'; target: DocTarget; view: DocViewSpec; expectSource?: string }
  /**
   * 프로젝트 자료 변경(같은 문서 D8, 원격은 프로젝트 폴더 안 경로의 추가·빼기·다시 읽기만). add 는 프로젝트 폴더 안 경로면 참조로, 바깥 경로면 사본으로 두고(이 컴퓨터 창만),
   * 프로젝트당 PROJECT_SOURCE_LIMIT 개까지다. 응답은 바뀐 목록(ProjectSourceList). reread 는 보관을 건너뛰고 새로 읽는다.
   */
  | { kind: 'source-change'; action: 'add'; projectId: string; path: string }
  | { kind: 'source-change'; action: 'remove' | 'reread'; projectId: string; sourceId: string }
  /** 이 대화에서 프로젝트 자료를 넣지 않는다(off=true) 또는 다시 넣는다. 다음 요청부터 적용한다. 응답은 Snapshot. */
  | { kind: 'set-session-sources'; sessionId: string; off: boolean }
  /** 이 대화의 웹 검색 정책을 바꾼다(null 이면 기본: 켬·범위 없음). 다음 요청부터 적용한다. 응답은 Snapshot. */
  | { kind: 'set-session-web'; sessionId: string; web: SessionWebPolicy | null }
  /** 이 대화의 공통 기억을 끄거나 켠다. 다음 요청부터 적용한다. 응답은 Snapshot(docs/notes/common-memory-2026-10-03.md M5). */
  | { kind: 'set-session-memory'; sessionId: string; off: boolean }
  | { kind: 'end-temporary'; sessionId: string; stop?: true }
  /** 방금 지운 대화를 되살린다(docs/notes/delete-undo-2026-09-26.md 1번). 마지막으로 지운 하나만, 12초 안에만 된다. */
  | { kind: 'undo-delete-session'; sessionId: string }
  | SessionParentRequest
  | MultiSendRequest
  /** 기록 동기화(docs/notes/encrypted-sync-2026-10-04.md S8). sync-probe 응답은 SyncProbeResult, 나머지는 Snapshot. */
  | SyncRequest
  /** 다른 기기에서 이어 가던 대화를 이 기기로 가져온다(S5). projectPath 는 사용자가 고른 이 기기의 작업 폴더. 응답은 Snapshot. */
  | { kind: 'session-take-over'; sessionId: string; requestId: string; projectPath: string }
  | { kind: 'mark-all-seen' }
  // target: 'host' 는 원격 창이 연결한 컴퓨터 알림을 바꿀 때.
  | { kind: 'set-notifications'; os: boolean; target?: 'host' }
  | ConversationTagRequest
  | ReplySummaryRequest
  | PaneSetRequest
  | ProjectGroupRequest
  | { kind: 'set-tool-ask'; mode: 'auto' | 'ask' }
  /** AI 가 앱 기능을 도구로 쓰게 할지 켜고 끈다(docs/notes/app-actions-2026-10-13.md 논쟁 9). AI 가 보낸 요청에서는 닫힌다. */
  | { kind: 'set-app-actions'; enabled: boolean }
  | SetNewChatDefaultRequest
  | SetEngineEnabledRequest
  | { kind: 'set-new-chat-approval'; approval: ApprovalMode; consent?: boolean; by?: string }
  /** 전체 허용 최초 동의를 기록한다. 이미 했으면 그대로 둔다(docs/notes/effort-approval-2026-10-01.md 5.2). by는 문이 정한다. */
  | { kind: 'set-approval-consent'; by?: string }
  | ({ kind: 'set-auto-compact' } & SetAutoCompactBody)
  | AppSettingRequest
  /** 설정 "동시에 도는 하위 대화" 수(같은 부모 기준, 정수 1~8). 올리면 기다리던 하위가 바로 돌고 내리면 도는 하위는 그대로 두고 새 시작만 기다린다(docs/notes/sub-sessions-2-2026-10-19.md 4.2). */
  | { kind: 'set-sub-session-running'; limit: number }
  /** 설정 "멈췄을 때" 전체를 한 번에 바꾼다. 그 설정에 기대던 이어 가기 예약은 지운다(docs/notes/failover-impl-2026-10-05.md 논쟁 필수 9). */
  | { kind: 'set-recovery'; recovery: RecoverySettings }
  /** 예약된 이어 가기를 지금 시작하거나 취소한다(같은 문서 R9). turnId 는 멈춘 요청이다. */
  | { kind: 'recovery-now'; sessionId: string; turnId: string }
  | { kind: 'recovery-cancel'; sessionId: string; turnId: string }
  /** 한도 결과 카드의 "다른 계정으로 이어 가기". 같은 엔진의 고른 계정(null=기본 로그인)으로 대화를 바꾸고 이어 가기 요청을 보낸다. 사용자가 누를 때만. */
  | { kind: 'continue-with-account'; sessionId: string; turnId: string; account: string | null }
  /** 설정 「일반」의 답 언어(docs/notes/answer-language-2026-10-02.md L1). 다음 요청부터 적용한다. */
  | { kind: 'set-answer-language'; value: AnswerLanguageSetting }
  | AnswerDetailRequest
  /** 설정 「진행 줄 안내 넣기」. 다음 요청부터 적용한다(docs/notes/answer-language-2026-10-02.md L4). */
  /** 설정 "내 Claude Code MCP도 쓰기". 다음 요청부터 적용한다(docs/notes/engine-mcp-2026-09-26.md 3번). */
  | { kind: 'set-claude-user-mcp'; enabled: boolean }
  /** MCP 연결 목록(docs/notes/mcp-registry-2026-10-04.md). 일반 관문을 따른다(보기 전용·참여자 거절, 전체 권한 원격 허용, 화면은 원격 창에서 잠근다). */
  | { kind: 'set-codex-user-mcp'; enabled: boolean }
  | McpRequest
  | ScheduleRequest
  /** 공유 링크(docs/notes/share-links-2026-10-04.md L8). preview 가 고정 사본(초안)을 만들고 create 는 그 초안만 잠근다. now 는 그 요청 뒤 판이 없을 때 지금 내용을 담겠다는 선택. */
  | { kind: 'share-preview'; sessionId: string; turnIds: string[]; files: Array<{ turnId: string; path: string; now?: true }> }
  /** mask false 면 confirmRaw true 가 있어야 한다. 같은 draftId 로 다시 오면 이미 만든 공유를 돌려준다. */
  | { kind: 'share-create'; draftId: string; expiryDays: ShareExpiryDays; mask: boolean; confirmRaw?: true }
  | { kind: 'share-list'; sessionId?: string }
  | { kind: 'share-revoke'; id: string }
  /**
   * 이슈 출처 연결 저장·끊기(10b·N11). 이 컴퓨터의 Lair 앱 창만 받는다. 토큰은 먼저 mcpSecret.set('issue/linear/key' | 'issue/jira/token') 으로 넣고,
   * 저장 요청이 그 토큰으로 연결을 확인한 뒤에만 저장한다. 응답은 스냅숏(실패면 사람 말 Error).
   */
  | { kind: 'issue-source-save'; source: 'linear' }
  | { kind: 'issue-source-save'; source: 'jira'; jiraKind: 'cloud' | 'datacenter'; site: string; email?: string }
  | { kind: 'issue-source-remove'; source: ExtIssueSource }
  /** 예약 사전 검사 명령 원문(N3). 이 컴퓨터의 Lair 앱 창만 받는다. */
  | { kind: 'schedule-precheck-get'; id: string }
  /** 원격 비밀 넣기(remote-accounts-mcp-skills D2). 값은 이 컴퓨터 보안 저장소에만 두고 응답·오류·기록에 싣지 않는다. 빈 값은 지움. 응답은 스냅숏. */
  | { kind: 'secret-store'; target: 'provider'; accountId: string; value: string }
  | { kind: 'secret-store'; target: 'mcp'; key: string; value: string }
  /** 이 대화에서 앱 MCP 서버 하나를 끄거나 켠다(D5). 원격에서도 된다. */
  | { kind: 'set-session-mcp'; sessionId: string; serverId: string; off: boolean }
  /**
   * 스킬(docs/notes/skills-2026-10-04.md R1~R10). skills-list 는 보기 요청이지만 원격에서 오면 엔진을 띄우지 않고 마지막 목록 사본만 준다(R8).
   * skill-read·skill-set-off 는 이 컴퓨터의 Lair 앱 창만 받는다. skill-set-off 는 서버가 목록 항목에서 키와 끌 수 있는지를 정한다(R9).
   */
  | { kind: 'skills-list'; projectId?: string; refresh?: boolean; language?: UiLanguage }
  | { kind: 'skill-read'; listId: string; id: string }
  | { kind: 'skill-set-off'; listId: string; id: string; engine: SkillEngine; off: boolean; language?: UiLanguage }
  /**
   * 스킬 설명 번역(docs/notes/skill-translation-2026-10-05.md T1~T6). plan 은 번역 없는 설명 수·보낼 글자 수·쓸 대상을 돌려주고(엔진을 부르지 않는다),
   * run 은 확인 뒤 그 대상의 생성 모드 작업자(도구 없음)로 번역해 설정에 저장한다. stop 이 참이면 도는 번역을 멈춘다. 둘 다 이 컴퓨터의 Lair 앱 창만 받는다.
   */
  | { kind: 'skill-translate-plan'; listId: string; language: UiLanguage }
  | { kind: 'skill-translate-run'; listId: string; language: UiLanguage; stop?: boolean; target?: SkillTranslateTarget; fingerprint?: string }
  /**
   * Lair 에 직접 추가한 스킬(docs/notes/mcp-skills-minimal-2026-10-10.md, 전역 docs/notes/skills-global-b762-2026-11-09.md). 엔진 목록 scan 없음.
   * list 는 includeGlobal:true 일 때만 전역을 싣고(옛 화면·폰은 안 보내 프로젝트 기록만 받는다), projectId 가 있으면 그 프로젝트도 싣는다. 둘 다 없으면 거절.
   * register 의 scope 'global' 은 모든 프로젝트·Claude·Codex 계정, 없으면 project(projectId 필수). set-enabled·delete 의 projectId 는 선택이고 전역은 번호로 찾는다.
   */
  | { kind: 'lair-custom-skills-list'; projectId?: string; includeGlobal?: boolean }
  | { kind: 'lair-custom-skill-register'; scope?: 'global' | 'project'; projectId?: string; sourceFolder: string; engines: Array<'claude' | 'codex'> }
  | { kind: 'lair-custom-skill-set-enabled'; projectId?: string; id: string; enabled: boolean }
  | { kind: 'lair-custom-skill-delete'; projectId?: string; id: string }
  | { kind: 'skills-off-list' }
  | { kind: 'skills-off-restore'; engine: SkillEngine; key: string }
  | { kind: 'resolve-unknown'; sessionId: string; turnId: string }
  /** 입력칸 "/compact". draftRevision이 맞고 초안이 "/compact"면 같은 저장에서 초안을 비운다(docs/notes/compaction-2026-09-24.md 1번). */
  | { kind: 'compact'; sessionId: string; requestId: string; draftRevision?: number }
  | { kind: 'summarize-handoff'; sessionId: string; requestId: string }
  /** afterTurnId·askId는 확인창을 연 순간의 마지막 요청·권함 번호, sendNext는 권함의 next 를 새로 시작 뒤 바로 보내기(docs/notes/restart-continue-2026-10-02.md D5). 문서는 120개까지. */
  | { kind: 'restart-session'; sessionId: string; requestId: string; documents: string[]; afterTurnId?: string; askId?: string; sendNext?: boolean }
  /**
   * 위임 켜기·끄기와 대상 엔진·모델(docs/notes/delegation-2026-09-24.md 3번, docs/notes/grok-worker-2026-09-26.md 1번).
   * 켤 때는 화면이 동의를 받은 뒤 보낸다. 엔진을 바꿀 때는 모델도 함께 보낸다.
   */
  | { kind: 'set-delegation'; enabled: boolean; model?: string; engine?: EngineId; /** 화면이 문구를 보여 사용자가 명시로 수락한 명령 위임 동의 판(docs/notes/claude-background-2026-10-01.md 9번). 이 칸이 있을 때만 판을 적는다. */ acceptCommandConsent?: number; /** 셸에서 쓰는 위임 스크립트 이름 전체(B-51). 이 칸이 있을 때만 바꾼다. */ shellScripts?: string[] }
  /**
   * 역할별 위임 정책 저장(docs/notes/role-policy-2026-10-05.md 설계 8절). policy 가 null 이면 그 범위의 정책을 지운다.
   * project 는 projectId, session 은 sessionId 가 있어야 한다. 바뀐 정책은 다음 요청부터 적용한다(논쟁 4번).
   */
  | { kind: 'set-role-policy'; scope: 'global' | 'project' | 'session'; projectId?: string; sessionId?: string; policy: RolePolicy | null;
    /** 편집기가 초안을 읽을 때의 판. 있으면 저장 직전 대조하고 다르면 거절한다. 빠뜨린 내부 경로(빠른 설정·설정 도우미)는 대조하지 않는다. */
    expectedRevision?: string }
  /**
   * AI와 설정하기 대화 열기(docs/notes/settings-assistant-2026-10-05.md 설계 4절). fromSessionId 가 있으면 그 대화·프로젝트 범위 항목도 다룬다.
   * 같은 fromSessionId 로 연 도우미 대화 중 실행 중이 아닌 가장 최근 것이 있으면 그것을 연다. 응답 Snapshot 의 session 이 도우미 대화다.
   */
  | { kind: 'open-settings-assistant'; fromSessionId?: string }
  /**
   * 설정 제안 카드 동작(설계 5절). apply 는 items(카드 안 번호) 필수이고 주의 항목은 혼자만 적용한다. revert 는 적용한 항목 전부를 한 번에 되돌린다.
   * decline 은 대기 중인 항목 전부를 접는다. 각 항목은 그 항목의 기존 설정 요청을 같은 검사·출처로 보내 바꾼다.
   */
  | { kind: 'settings-proposal'; action: 'apply' | 'revert' | 'decline'; sessionId: string; proposalId: string; items?: number[];
      /** 요청 관문만 채운다(논쟁 F1). 원격이면 기기 이름과 폰 여부, 이 컴퓨터 창이면 '이 컴퓨터'. 화면이 보낸 값은 관문이 지운다. */
      by?: string; remoteDevice?: 'phone' | 'other' }
  /** 첫 실행 질문지(설계 6절). answers 에 든 문항만 적용하고 답한 문항으로 적는다. skip 이면 남은 문항을 다시 묻지 않는다. 본인 전체 권한 원격도 진행한다(10-07 사용자 결정). by 는 요청 관문만 채운다(원격이면 기기 이름, 이 컴퓨터 창이면 '이 컴퓨터'). */
  | { kind: 'quick-setup'; answers?: QuickSetupAnswers; skip?: boolean; by?: string }
  /**
   * 로컬 모델 공급처 목록 저장(docs/notes/acp-engines-2026-10-05.md 설계 5절). providers 가 null 이면 저장한 목록을 지워 기본 주소 셋으로 돌아간다.
   * 저장 뒤 공급처를 다시 확인하고 OpenCode 상태를 다시 본다. 앱이 이 주소로 요청을 보내므로 이 컴퓨터의 창에서만 받는다.
   */
  | { kind: 'set-local-providers'; providers: LocalProviderSetting[] | null }
  /** 로컬 모델 공급처를 다시 확인한다(모델 목록·문맥). 이 컴퓨터의 창에서만 받는다. */
  | { kind: 'check-local-providers' }
  | { kind: 'stop-delegation'; sessionId: string; delegationId: string }
  /** 끝난(중단·실패·시간 초과) 위임을 같은 지시로 새 위임으로 다시 맡긴다(F9 f-3). 이 컴퓨터의 창에서만 받는다. */
  | { kind: 'delegation-retry'; sessionId: string; delegationId: string }
  /** 결과 불명 위임을 사용자가 직접 끝난 것을 확인했다고 알린다. 상태는 불명 그대로 두고 다시 맡기기·새로 시작·쓰기 점유 제한만 푼다. 프로세스는 건드리지 않는다. delegation-retry 와 같은 규칙으로 이 컴퓨터의 창에서만 받는다. */
  | { kind: 'delegation-release'; sessionId: string; delegationId: string }
  | { kind: 'delegate-direct'; sessionId: string; clientRequestId: string; task: string; target?: DelegateTargetInput; role?: RoleId; mode?: WorkerMode; folder?: string; timeoutMinutes?: number; continue?: string }
  | { kind: 'delegation-approval'; sessionId: string; delegationId: string; approvalId: string; decision: 'accept' | 'decline' }
  /** 쓰기 위임 후보 적용 미리보기(보기 요청, P5). 응답 DelegationApplyPreview. */
  | { kind: 'delegation-view'; action: 'apply'; sessionId: string; delegationId: string; attemptId: string; skipConflicts?: boolean }
  /** 추가 지시 하나로 실제 보낸 위임 결과 글을 앱이 다시 조립해 돌려준다(화면 "넘긴 내용 보기"와 AI 읽기 공용, 응답 { text, blocks, missing? }). */
  | { kind: 'delegation-steer-text'; sessionId: string; turnId: string; steerId: string }
  /** 쓰기 위임 적용(version 필수)·적용 되돌리기·모두 버리기(W5·P5·P6). 응답 DelegationChangeResult. */
  | { kind: 'delegation-change'; action: 'apply'; sessionId: string; delegationId: string; attemptId: string; version: string; skipConflicts?: boolean }
  | { kind: 'delegation-change'; action: 'undo'; sessionId: string; delegationId: string }
  | { kind: 'delegation-change'; action: 'discard'; sessionId: string; delegationId: string }
  | { kind: 'delegation-change'; action: 'schedule-apply'; sessionId: string; delegationId: string; attemptId: string }
  | { kind: 'delegation-change'; action: 'cancel-schedule'; sessionId: string; delegationId: string }
  | { kind: 'stop'; sessionId: string; /** 관문만 싣는다(참여자 중지, R6). */ participant?: ParticipantProjection }
  /** 터미널 명령 lair status·list 용 대화 목록·상태 개수 읽기(docs/notes/lair-command-2026-10-19.md I3). 이 컴퓨터의 앱 창만 받는다. 답은 CliOverview. */
  | CliOverviewRequest
  /** 바깥 터미널 엔진 대화 이어 열기: 출처 보기·대화 목록(보기)과 Lair 대화로 열기(바꾸기, mode same 은 사람 확인이 필요해 AI 는 fork 만). 이 컴퓨터 화면과 전체 권한 원격만 받는다(docs/notes/engine-import-2026-10-19.md). 답은 EngineImportSourcesResult·EngineImportListResult·EngineImportOpenResult. */
  | EngineImportRequest
  | MigrateRequest
  /**
   * 참여자 자기 API 키 넣기·빼기(docs/notes/participant-runs-2026-10-04.md R2·R6). 참여자 기기에서만 오고 주 프로세스 관문이 처리한다(실행부에 오지 않는다).
   * key 가 null 이면 뺀다. 응답은 참여자 투영 snapshot 이다.
   */
  | { kind: 'participant-key'; provider: ParticipantKeyProvider; key: string | null };

export interface LairBridge extends DelegationBridge, RecordsBridge, BoardClipBridge, BoardDragBridge, WindowChromeBridge, SessionReadBridge<SessionState, Turn, Delegation, Compaction> {
  /** 이 화면이 뜰 때 정해진 화면 언어(다국어-1, docs/notes/i18n-base-2026-10-04.md 공유 약속). 데스크톱은 주 프로세스, 웹은 그 브라우저 값. */
  uiLanguage: UiLanguage;
  uiLanguageSetting: UiLanguageSetting;
  /** 설정이 'system' 일 때 쓰일 언어(운영체제·브라우저 언어 목록에서 풀이). 설정 줄의 "시스템 언어 따라(지금: …)" 표시용. */
  uiLanguageSystem: UiLanguage;
  /** 화면 언어를 저장하고 닫기 준비를 거쳐 화면을 다시 불러온다. 준비가 거절되면 저장만 하고 applied 거짓. */
  /** reload 는 이 화면이 곧 다시 불러와지는지(풀이한 언어가 같으면 거짓 — 이때 화면은 일회 복구 표지를 바로 지운다). */
  setUiLanguage(setting: UiLanguageSetting): Promise<{ applied: boolean; reload: boolean; message?: string }>;
  /** 웹 전용 선택 칸. 데스크톱에는 없다. */
  webConnection?: { get(): WebConnectionState; subscribe(listener: (state: WebConnectionState) => void): () => void };
  request(request: Extract<AppRequest, { kind: 'request-receipt' }>): Promise<RequestReceiptView>;
  request(request: Extract<AppRequest, { kind: 'engine-path' }>): Promise<EnginePathResult>;
  request(request: Extract<AppRequest, { kind: 'account-reset-credit' }>): Promise<ResetCreditResult>;
  request(request: Extract<AppRequest, { kind: 'list-prepared' | 'list-prepared-all' }>): Promise<RequestReceiptView[]>;
  request(request: Extract<AppRequest, { kind: 'discard-prepared' | 'dismiss-prepared' }>): Promise<PreparedChangeResult>;
  request(request: Extract<AppRequest, { kind: 'inspect-imported-folders' }>): Promise<ImportedFoldersInspection>;
  request(request: Extract<AppRequest, { kind: 'map-imported-folders' }>): Promise<ImportedFoldersMapResult>;
  request(request: Extract<AppRequest, { kind: 'list-chat-folder-projects' }>): Promise<ChatFolderProjectsInspection>;
  request(request: Extract<AppRequest, { kind: 'worktree-status' }>): Promise<WorktreeStatus>;
  request(request: Extract<AppRequest, { kind: 'folder-status' }>): Promise<FolderStatus | null>;
  request(request: Extract<AppRequest, { kind: 'inspect-folder-scope' }>): Promise<FolderScopeInfo>;
  request(request: Extract<AppRequest, { kind: 'browser-capture' }>): Promise<{ dataUrl: string }>;
  request(request: Extract<AppRequest, { kind: 'session-images' }>): Promise<SessionImageItem[]>;
  request(request: Extract<AppRequest, { kind: 'session-children' }>): Promise<SubSessionChild[]>;
  request(request: Extract<AppRequest, { kind: 'multi-stage-add' | 'multi-stage-remove' | 'multi-stage-clear' }>): Promise<StagedFile[]>;
  request(request: Extract<AppRequest, { kind: 'multi-send' }>): Promise<MultiSendResult>;
  request(request: Extract<AppRequest, { kind: 'worktree-prep-view' | 'worktree-prep' }>): Promise<WorktreePrepResult>;
  request(request: Extract<AppRequest, { kind: 'branch-view' | 'branch-change' }>): Promise<BranchResult>;
  request(request: Extract<AppRequest, { kind: 'worktree-merge-view'; action: 'merge' }>): Promise<WorktreeMergePreview>;
  request(request: Extract<AppRequest, { kind: 'worktree-merge-view'; action: 'clean' }>): Promise<WorktreeCleanPreview>;
  request(request: Extract<AppRequest, { kind: 'worktree-merge' }>): Promise<WorktreeMergeResult>;
  request(request: Extract<AppRequest, { kind: 'copy-view'; action: 'apply' }>): Promise<CopyApplyPreview>;
  request(request: Extract<AppRequest, { kind: 'copy-view'; action: 'clean' }>): Promise<CopyCleanPreview>;
  request(request: Extract<AppRequest, { kind: 'copy-change' }>): Promise<CopyChangeResult>;
  request(request: Extract<AppRequest, { kind: 'delegation-view' }>): Promise<DelegationApplyPreview>;
  request(request: Extract<AppRequest, { kind: 'delegation-change' }>): Promise<DelegationChangeResult>;
  request(request: Extract<AppRequest, { kind: 'delegation-steer-text' }>): Promise<DelegationSteerText>;
  request(request: Extract<AppRequest, { kind: 'restore-plan' }>): Promise<RestorePlan>;
  request(request: Extract<AppRequest, { kind: 'sync-probe' }>): Promise<SyncProbeResult>;
  request(request: Extract<AppRequest, { kind: 'restore-run' | 'restore-undo' | 'restore-trashed' }>): Promise<RestoreRunResult>;
  request(request: Extract<AppRequest, { kind: 'source-view'; action: 'list' } | { kind: 'source-change' }>): Promise<ProjectSourceList>;
  request(request: Extract<AppRequest, { kind: 'source-view'; action: 'evidence' }>): Promise<EvidenceOpen>;
  request(request: Extract<AppRequest, { kind: 'source-view'; action: 'document' }>): Promise<DocumentView>;
  request(request: Extract<AppRequest, { kind: 'share-preview' }>): Promise<ShareDraft>;
  request(request: Extract<AppRequest, { kind: 'share-create' }>): Promise<ShareItem>;
  request(request: Extract<AppRequest, { kind: 'share-list' | 'share-revoke' }>): Promise<ShareList>;
  request(request: BrowserOriginsRequest): Promise<BrowserOriginsResult>;
  request(request: Extract<CommandAllowRequest, { kind: 'command-allows-list' }>): Promise<CommandAllowList>;
  request(request: Extract<AppRequest, { kind: 'engine-import-sources' }>): Promise<EngineImportSourcesResult>;
  request(request: Extract<AppRequest, { kind: 'engine-import-list' }>): Promise<EngineImportListResult>;
  request(request: Extract<AppRequest, { kind: 'engine-import-open' }>): Promise<EngineImportOpenResult>;
  request(request: Extract<AppRequest, { kind: MigrateRequest['kind'] }>): Promise<MigrateResult | Snapshot>;
  request(request: Exclude<AppRequest,{ kind: 'request-receipt' } | { kind: 'engine-path' } | { kind: 'list-prepared' } | { kind: 'list-prepared-all' } | { kind: 'discard-prepared' } | { kind: 'dismiss-prepared' } | { kind: 'inspect-imported-folders' } | { kind: 'map-imported-folders' } | { kind: 'list-chat-folder-projects' } | { kind: 'worktree-status' } | { kind: 'folder-status' } | { kind: 'browser-capture' } | { kind: 'worktree-prep-view' } | { kind: 'worktree-prep' } | { kind: 'branch-view' } | { kind: 'branch-change' } | { kind: 'worktree-merge-view' } | { kind: 'worktree-merge' } | { kind: 'copy-view' } | { kind: 'copy-change' } | { kind: 'delegation-view' } | { kind: 'delegation-change' } | { kind: 'delegation-steer-text' } | { kind: 'restore-plan' } | { kind: 'restore-run' } | { kind: 'restore-undo' } | { kind: 'restore-trashed' } | { kind: 'source-view' } | { kind: 'source-change' } | { kind: 'share-preview' } | { kind: 'share-create' } | { kind: 'share-list' } | { kind: 'share-revoke' } | { kind: 'browser-origins' } | { kind: 'command-allows-list' }>): Promise<Snapshot>;
  /** 음성 받아쓰기(docs/notes/voice-dictation-2026-09-24.md). 내려받기는 시작만 하고 진행은 onSpeechStatus로 알린다. */
  speechStatus(): Promise<SpeechStatus>;
  speechDownload(): Promise<SpeechStatus>;
  speechCancelDownload(): Promise<SpeechStatus>;
  /** 원격 창에서 받아쓰는 컴퓨터를 고른다(부록 A6). 로컬 창에서는 거절한다. */
  speechSetRunner(choice: SpeechRunnerChoice): Promise<SpeechStatus>;
  speechDelete(): Promise<SpeechStatus>;
  /** 연결 계정 키(docs/notes/provider-accounts-2026-10-03.md D2). 이 컴퓨터의 Lair 앱 창만 쓴다. 키는 lair:request 를 거치지 않는다. */
  providerKey: {
    status(): Promise<{ available: boolean; reason?: string }>;
    set(accountId: string, key: string): Promise<{ ok: true }>;
    clear(accountId: string): Promise<{ ok: true }>;
  };
  /** MCP 비밀 값(docs/notes/mcp-registry-2026-10-04.md R6). 이 컴퓨터의 Lair 앱 창만 쓴다. 값은 lair:request 를 거치지 않는다. key 는 `<서버 id>/<env|header>/<이름>`. */
  mcpSecret: {
    status(): Promise<{ available: boolean; reason?: string }>;
    set(key: string, value: string): Promise<{ ok: true }>;
    clear(key: string): Promise<{ ok: true }>;
  };
  speechSetLanguage(language: SpeechLanguage): Promise<SpeechStatus>;
  /** 녹음을 시작할 때 모델을 미리 올린다. */
  speechPrepare(): Promise<void>;
  /** 16kHz 단일 채널 16비트 PCM 조각 하나를 받아쓴다. */
  speechTranscribe(pcm: ArrayBuffer, prompt: string, onPartial?: (text: string) => void): Promise<{ text: string; ms: number }>;
  /** 받아쓴 글을 클립보드에 둔다(녹음 중 대화가 바뀐 경우). */
  speechCopy(text: string): Promise<void>;
  onSpeechStatus(listener: (status: SpeechStatus) => void): () => void;
  /** purpose 'browse' 는 파일함 다른 폴더 보기(창 제목만 다르다, 묶음 3). */
  chooseFolder(purpose?: 'browse'): Promise<string | null>;
  /** 엔진 실행 파일 하나를 고른다(데스크톱만, docs/notes/engine-binary-path-2026-10-12.md D5). 링크·별칭을 풀지 않은 경로, 취소하면 null. 원격 창·웹은 쓰지 않는다. */
  chooseEngineBinary(): Promise<string | null>;
  chooseFiles(sessionId: string): Promise<Snapshot | null>;
  dropFiles(sessionId: string, files: File[]): Promise<Snapshot | null>;
  pasteImage(sessionId: string): Promise<Snapshot | null>;
  /** 여러 대화 보내기 스테이지에 파일을 올린다(docs/notes/multi-send-2026-11-02.md D3). 어느 대화 소유도 아니다. 취소하면 null, 아니면 스테이지 전체. */
  stageFiles(stageId: string, method: 'choose' | 'drop' | 'paste', files?: File[]): Promise<StagedFile[] | null>;
  /** 터미널 출력 같은 글을 텍스트 파일 사본으로 첨부한다(docs/notes/terminal-slice3-2026-09-27.md 4번). */
  attachText(sessionId: string, name: string, text: string): Promise<Snapshot | null>;
  saveDraft(sessionId: string, text: string, expectedRevision: number): Promise<Draft>;
  /** 메모장: 목록(제목만, 최근 고친 순), 한 건 읽기, 저장(id 가 없으면 새 메모, expectedRevision 은 새 메모면 0), 삭제, 검색(제목·본문, 최대 50건). */
  listNotes(): Promise<NoteSummary[]>;
  readNote(id: string): Promise<Note | null>;
  saveNote(id: string | null, body: string, expectedRevision: number): Promise<Note>;
  deleteNote(id: string): Promise<void>;
  searchNotes(query: string): Promise<NoteSearchHit[]>;
  /** 메모 하나를 .md 파일로 내보낸다. 데스크톱은 저장 창(취소면 null), 웹은 브라우저 내려받기. */
  exportNote(id: string): Promise<FileExport | null>;
  /**
   * 캡처 보드(docs/notes/capture-board-2026-10-14.md). 채널은 'lair:board-…'. 목록은 하나도 없으면 실행부가 생성 시 언어에 맞는 기본 이름의 보드를 만들어 돌려준다.
   * 읽기는 메타데이터만(그림 바이트 없음). 위치 저장은 카드 단위 rev 로 충돌을 본다(충돌한 카드만 실행부 값으로 돌려준다).
   */
  listBoards(language?: UiLanguage): Promise<BoardSummary[]>;
  readBoard(boardId: string): Promise<Board | null>;
  createBoard(name?: string, language?: UiLanguage): Promise<BoardSummary>;
  renameBoard(boardId: string, name: string): Promise<void>;
  duplicateBoard(boardId: string, language?: UiLanguage): Promise<void>;
  deleteBoard(boardId: string): Promise<void>;
  addBoardText(boardId: string, input: BoardTextInput): Promise<BoardCard>;
  addBoardImage(boardId: string, input: BoardImageInput): Promise<BoardCard>;
  moveBoardCards(boardId: string, cards: BoardPlacement[]): Promise<BoardMoveResult>;
  /** 나중 저장이 이긴다(rev 비교 없음). */
  saveBoardText(cardId: string, text: string): Promise<BoardCard>;
  /** deleted_at 만 찍는다. 파일은 10분 뒤 정리한다. */
  deleteBoardCards(boardId: string, cardIds: string[]): Promise<{ revision: number }>;
  /** 되돌리기. 카드 상한을 다시 검사한다. */
  restoreBoardCards(boardId: string, cardIds: string[]): Promise<BoardCard[]>;
  /** 파일은 그대로 두고 board_id 만 바꾼다. 카드 상한을 검사한다. */
  transferBoardCards(fromBoardId: string, toBoardId: string, cardIds: string[]): Promise<{ revision: number }>;
  /** 그림 카드 원본을 첨부 규칙대로 복사해 대기 첨부로 넣는다(재인코딩 없음). 글 카드는 무시한다. 순서는 cardIds 순서. */
  boardAttach(sessionId: string, cardIds: string[]): Promise<BoardAttachResult>;
  /**
   * 화면이 만든 PNG 한 장(‘한 장으로 합쳐 첨부’, 첨부 상한을 넘는 카드의 축소본)을 대기 첨부로 넣는다. 데스크톱은 실행부 importImage(붙여넣기와 같은 검사),
   * 웹·연결 창은 기존 파일 올리기 통로로 보낸다. 실제 파일 경로가 없는 그림이라 dropFiles 를 쓰지 않는다.
   */
  boardAttachImage(sessionId: string, png: Uint8Array, name: string): Promise<Snapshot | null>;
  /**
   * 데스크톱·연결한 창·웹(보기 전용 제외). 썸네일이 없는 그림 카드(Lair 브라우저 캡처 등)의 썸네일을 화면이 원본에서 만들어 저장한다(WebP, 긴 변 BOARD_THUMB_SIDE 이하).
   * 원본과 같은 장면에서 만든 썸네일만 쓰려고 캡처 때 따로 찍지 않는다(1a 감사 필수 5). 판이 다르거나 이미 있으면 그대로 돌려준다.
   */
  setBoardThumb?(cardId: string, version: number, thumb: Uint8Array): Promise<BoardCard>;
  /** 동기. 데스크톱 lair-board://c/<cardId>/<version>/<size>, 웹 /lair/board/<cardId>/<version>/<size>. */
  boardImageUrl(cardId: string, version: number, size: import('./board-video').BoardFileVariant): string;
  /** 데스크톱만(웹에는 없다. 연결한 창은 그 기기의 브라우저). 그 창 Lair 브라우저의 지금 탭 보이는 영역을 보드 카드로 넣는다. */
  boardCaptureBrowser?(boardId: string): Promise<BoardCard>;
  /** 데스크톱만(연결한 창은 그 기기). 기존 영역 고르기를 시작하되 결과를 보드에 넣는다(끝나면 onBoardsChanged 로 알게 된다). */
  boardPickBrowserArea?(boardId: string): Promise<void>;
  /** 데스크톱만(연결한 창은 boardPrepareDrag 로 받아 둔 카드만). 고른 그림 카드를 다른 앱으로 끌어낸다(preload 의 send, 동기 처리 없음). */
  /** icon: 마우스 옆에 보일 작은 PNG data URL. 없으면 기본 아이콘. */
  startBoardDrag?(items: { cardId: string; version: number }[], icon?: string): void;
  /**
   * 데스크톱만(연결한 창은 그 기기, 1b 운영체제 캡처, docs/notes/capture-board-2026-10-14.md 1b 구현 계약). 영역은 고르기 창, 창은 sourceId(boardListWindows 의 id,
   * Wayland 는 생략해 운영체제 고르기 창), 전체 화면은 커서가 있는 화면. Lair 창은 숨기지 않고 보이는 화면 그대로 찍는다.
   */
  boardCaptureScreen?(boardId: string, kind: BoardScreenCaptureKind, sourceId?: string): Promise<BoardScreenCaptureResult>;
  /** 이 컴퓨터 데스크톱만. 창 캡처 고르기 목록(열 때 한 번, 캐시 없음). Wayland 는 { picker: 'system' }(목록 대신 운영체제 고르기 창). */
  boardListWindows?(forRecord?: boolean): Promise<BoardWindowList>;
  /** 이 컴퓨터 데스크톱만. 화면 기록 권한 다시 확인(macOS 밖은 늘 granted). 주기 확인 없음. */
  boardCapturePermission?(): Promise<ScreenCapturePermission>;
  /** 이 컴퓨터 데스크톱만. 운영체제의 화면 기록 권한 설정을 연다(macOS). */
  openBoardCapturePermission?(): Promise<void>;
  /** 이 컴퓨터 데스크톱만. 앱 밖 단축키의 대상 보드(마지막으로 연 보드)를 주 프로세스에 알린다(send, 응답 없음). */
  setBoardCaptureTarget?(boardId: string): void;
  /** 이 컴퓨터 데스크톱만. 앱 밖 캡처 단축키(이 기기 설정, <앱 데이터>/capture-shortcut.json). */
  captureShortcut?: CaptureShortcutBridge;
  /** 단축키 설정(이 기기 설정. 데스크톱은 <앱 데이터>/keybindings.json, 웹은 이 브라우저 저장소). */
  keybindings: KeybindingsBridge;
  onPrepareClose(listener: () => Promise<void>): () => void;
  previewFile(target: FileTarget): Promise<FilePreview>;
  /**
   * 파일함 그리드 카드의 가운데 정사각형 썸네일 데이터 URL(docs/notes/file-library-polish-2026-09-25.md 2번). 만들지 못하면 null이다.
   * 소리 파일은 파일에 든 표지 그림 그대로다. 정사각형으로 줄이는 것은 화면이 한다(docs/notes/audio-preview-2026-09-25.md 2번).
   */
  fileThumbnail(target: FileTarget): Promise<string | null>;
  /** 영상·소리 재생 주소(docs/notes/video-preview-2026-09-25.md 1번). 영상·소리가 아니거나 확인하지 못하면 null이다. */
  fileMedia(target: FileTarget): Promise<string | null>;
  /** 재생 주소가 가리키는 소리 파일 내용. 파형을 그릴 때만 쓴다. 소리가 아니거나 64 MiB를 넘거나 파일이 바뀌었으면 null이다(docs/notes/audio-preview-2026-09-25.md 2번). */
  fileAudioData(url: string): Promise<Uint8Array | null>;
  /** HTML 캔버스 주소(docs/notes/html-canvas-2026-09-26.md). 열 때마다 새 번호이고 인터넷은 꺼진 채다. HTML이 아니거나 확인하지 못하면 null이다. */
  fileCanvas(target: FileTarget): Promise<string | null>;
  /** 캔버스 주소의 인터넷 켬·끔. 모르는 주소면 false다. */
  setCanvasInternet(address: string, on: boolean): Promise<boolean>;
  /** 캔버스 틀이 제 주소 밖으로 가려다 막혔을 때. 번호와 막은 주소(앞 300자)다. */
  onCanvasBlocked(listener: (event: { token: string; url: string }) => void): () => void;
  /** 요청 앞에 붙여 보낸 인계 글 전문(docs/notes/engine-switch-2026-09-24.md 8번). */
  readHandoff(sessionId: string, turnId: string): Promise<{ text: string }>;
  /** 엔진이 문맥 압축 뒤 이어 가는 요약 글 전문(docs/notes/compaction-2026-09-24.md 6번). */
  readCompaction(sessionId: string, compactionId: string): Promise<{ text: string }>;
  /** 대화 제목·작업 폴더와 요청·답·추가 지시·첨부·결과 파일·위임 지시문에서 찾는다(docs/notes/search-2026-09-24.md). */
  searchSessions(query: string): Promise<SearchResult>;
  /** 연 대화 안 찾기의 창 밖 요청 확인(docs/notes/find-in-chat-2026-09-24.md 6번). */
  findInSession(sessionId: string, query: string): Promise<FindResult>;
  readDelivery(sessionId: string, turnId: string): Promise<{ text: string }>;
  /** 그 요청에 엔진이 알린 규칙 파일 하나를 읽기 전용으로 연다(docs/notes/rules-2026-09-24.md 2번). */
  readRuleFile(sessionId: string, turnId: string, path: string): Promise<RuleFileView>;
  /** 작업 폴더 안 규칙 파일을 읽은 판(version)이 그대로일 때만 고친다(docs/notes/rules-edit-2026-09-26.md 4·5번). */
  writeRuleFile(sessionId: string, turnId: string, path: string, version: string, text: string): Promise<RuleFileSave>;
  /** 공통 규칙 읽기·저장하고 적용·밖에서 고친 구역 처리(docs/notes/common-rules-2026-09-26.md). */
  commonRules(action: CommonRulesAction): Promise<CommonRulesView>;
  /** Claude 자동 기억 목록·읽기·고치기·잊기(docs/notes/memory-edit-2026-09-26.md). */
  memory(action: MemoryAction): Promise<MemoryResult>;
  /** Lair 공통 기억 목록·고치기·후보 저장(docs/notes/common-memory-2026-10-03.md M4). Claude 자동 기억(memory)과 따로다. */
  commonMemory(action: CommonMemoryAction): Promise<CommonMemoryResult>;
  /** 프로젝트 지시 파일 목록·읽기·고치기(docs/notes/projects-slice1-2026-09-27.md 9번). */
  projectRules(action: ProjectRulesAction): Promise<ProjectRulesResult>;
  /** 변경 보기(docs/notes/projects-slice2-2026-09-27.md). */
  changes(action: ChangesAction | GithubAction | FileVersionAction): Promise<ChangesResult>;
  /** 이 컴퓨터에서 찾은 외부 편집기 */
  editors(): Promise<ExternalEditor[]>;
  /** 연 폴더 안 파일을 외부 편집기·기본 앱으로 열거나 Finder·탐색기에서 보인다. */
  openInEditor(request: { folder: string; path: string; editor: string; line?: number; source?: 'changes' | 'files' | 'path' }): Promise<void>;
  /** 프로젝트 파일(docs/notes/projects-slice3-2026-09-27.md). */
  projectFiles(action: ProjectFilesAction): Promise<ProjectFilesResult>;
  /** 메시지·문서 댓글(docs/notes/comments-coedit-2026-10-04.md C5). */
  comments(action: CommentsAction): Promise<CommentsResult>;
  /** 공유 프로젝트 파일의 공동 편집(같은 문서 C4·C5). */
  coedit(action: CoeditAction): Promise<CoeditResult>;
  /** 이 화면(창·기기)이 연 공동 문서의 사건 */
  onCoedit(listener: (event: CoeditEvent) => void): () => void;
  onCommentsChanged(listener: (event: CommentsChangedEvent) => void): () => void;
  saveFileCopy(target: FileTarget): Promise<FileExport | null>;
  /**
   * 그림 하나를 시스템 클립보드에 복사한다(데스크톱 전용, docs/notes/image-viewer-2026-10-13.md 4.7절 4번). 주 프로세스가 previewFile 과 같은 확인으로
   * 원본 바이트를 읽어 넣는다. 복사하면 true, 그림이 아니거나 읽지 못했거나 이 연결에서 바이트를 받을 수 없으면 false 다.
   * 웹과 다른 컴퓨터에 연결한 데스크톱 앱은 항상 false 다(웹 화면은 브라우저 ClipboardItem 으로 따로 복사한다).
   */
  copyImage(target: FileTarget): Promise<boolean>;
  /**
   * 브라우저 증거 캡처 하나를 시스템 클립보드에 복사한다(데스크톱 전용, docs/notes/image-viewer-2026-10-13.md 4.7절·9절). 주 프로세스가
   * 'browser-capture' 요청으로 원본을 받아 PNG 로 넣는다. 복사하면 true, 읽지 못했거나 그림이 아니면 false 다. 웹은 항상 false 다(화면이 브라우저 ClipboardItem 으로 복사한다).
   */
  copyCapture(ref: { sessionId: string; turnId: string; captureId: string }): Promise<boolean>;
  /**
   * 브라우저 증거 캡처 하나를 파일로 저장한다. 저장 창을 취소하면 없다. 웹은 브라우저 내려받기로 받는다.
   * suggestedName 은 저장 창의 기본 이름이다.
   */
  saveCapture(ref: { sessionId: string; turnId: string; captureId: string }, suggestedName: string): Promise<FileExport | null>;
  /** 결과물 판의 사본 저장(데스크톱 전용, docs/notes/result-versions-2026-10-03.md D2). 저장 창을 취소하면 없다. 웹은 지원하지 않는다. */
  saveVersionCopy(folder: string, path: string, id: string): Promise<FileExport | null>;
  /** 대화 하나를 Markdown 파일로 내보낸다. 저장 창을 취소하면 없다(docs/notes/export-restore-2026-09-24.md). */
  exportSession(sessionId: string): Promise<FileExport | null>;
  listWorkspaceFiles(scope: WorkspaceScope, query: string): Promise<WorkspaceListing>;
  /** 파일함 작업 폴더 보기가 열려 있는 동안 그 폴더를 지켜본다. null이면 멈춘다. 걸지 못하면 false다(docs/notes/file-library-refresh-2026-09-25.md 1번). */
  watchWorkspace(scope: WorkspaceScope | null): Promise<boolean>;
  /** 파일 창에 보이는 프로젝트 파일 하나를 지켜본다. null이면 멈춘다(docs/notes/file-panel-2026-09-27.md 6.2절 5번). */
  watchFile(target: { folder: string; path: string } | { file: FileTarget } | null): Promise<boolean>;
  listSessionFiles(sessionId: string | null, filter: SessionFileFilter): Promise<SessionFileList>;
  /** 고른 미사용 사본을 지운다. 한 번에 500개까지다(docs/notes/unused-cleanup-2026-09-26.md 3번). */
  removeUnusedFiles(targets: UnusedFileTarget[]): Promise<UnusedCleanup>;
  /** 복원 뒤 남은 옛 기록 폴더와 하나 지우기. 데이터 폴더는 주 프로세스만 안다(docs/notes/restore-leftovers-2026-09-26.md). */
  listRestoreLeftovers(): Promise<RestoreLeftover[]>;
  removeRestoreLeftover(name: string): Promise<{ bytes: number }>;
  /** 파일이 있는 대화와 대화별 파일 수. 파일함 범위 메뉴가 열 때마다 받는다. */
  listFileSessions(): Promise<SessionFileCount[]>;
  attachWorkspaceFiles(sessionId: string, paths: string[]): Promise<Snapshot | null>;
  /** 파일함 '다른 폴더' 보기: 고른 폴더(root) 안의 한 폴더 목록·이름 찾기. root는 이 컴퓨터 창이 고른 폴더여야 한다. */
  listFolder(request: { root: string; path?: string; query?: string }): Promise<FolderListing>;
  /** 다른 폴더 보기에서 고른 파일(root 안 일반 파일, 1~16개)을 대화에 붙인다. */
  attachFolderFiles(sessionId: string, root: string, paths: string[]): Promise<Snapshot | null>;
  restart(): Promise<void>;
  /** sessionId: 엔진 이벤트처럼 그 대화 하나만 바뀐 변경(docs/notes/multi-pane-streaming-2026-09-25.md 6번). 없으면 전체 변경이다. */
  onChanged(listener: (sessionId?: string, change?: import('./change-stamp').ChangeStamp, scope?: import('./change-stamp').ChangeScope) => void): () => void;
  /** 지켜보는 작업 폴더가 바뀌었다. stopped면 감시가 오류로 멈췄다. */
  onWorkspaceChanged(listener: (stopped: boolean) => void): () => void;
  /** 지켜보는 파일(절대 경로)이 바뀌었을 수 있다. 화면이 다시 읽어 판을 비교한다. */
  onFileChanged(listener: (path: string) => void): () => void;
  onBackendError(listener: (message: string, unavailable: boolean, kind?: 'update-updated') => void): () => void;
  /** 운영체제 알림을 눌러 온 이동. 화면이 준비되기 전에 온 것은 보관했다가 등록 때 넘긴다. */
  onOpenTarget(listener: (target: AttentionTarget) => void): () => void;
  /** 창이 앞에 있을 때 새로 생긴 확인할 일(창 안 알림용). */
  onAttention(listener: (event: AttentionEvent) => void): () => void;
  /** 메뉴 편집의 찾기 명령(docs/notes/find-in-chat-2026-09-24.md 2번). */
  onFind(listener: (action: FindAction) => void): () => void;
  /** 진단 자료 만들기와 저장(docs/notes/diagnostics-2026-09-24.md). 저장 창을 취소하면 없다. */
  buildDiagnostics(sections: DiagnosticsSection[]): Promise<Diagnostics>;
  saveDiagnostics(id: string): Promise<FileExport | null>;
  /**
   * 로그 폴더를 파일 관리자로 연다(docs/notes/app-log-2026-09-24.md 9번). 원격 창에서 which가 없으면 어느 로그인지 묻고, 'this'면 이 기기 로그 폴더,
   * 'host'면 호스트 로그를 이 기기에 사본으로 받는다(docs/notes/settings-view-2026-09-29.md 2.1절). 이 컴퓨터 창은 which를 보지 않는다.
   */
  openLogFolder(which?: 'this' | 'host'): Promise<void>;
  /** 글을 그대로 클립보드에 둔다. 앱이 웹 권한을 모두 거절해 화면의 복사 단추는 이것을 쓴다(docs/notes/copy-text-2026-09-25.md). */
  copyText(text: string): Promise<void>;
  /** 앱 메뉴 "진단 자료 만들기…"(같은 문서 1번) */
  onShowDiagnostics(listener: () => void): () => void;
  /** 옛 Lair 가져오기 결과 대화상자의 "경로 맞추기"(docs/notes/perf-2026-10-01.md 4절): "가져온 대화 경로 맞추기" 창을 연다. */
  onShowImportedFolders(listener: () => void): () => void;
  /** 운영체제. 머리줄을 창 제목줄 자리에 둘 때 창 단추 자리를 비킨다(docs/notes/titlebar-2026-09-25.md). */
  platform: string;
  /** 셸이 도는 컴퓨터의 운영체제. 원격 창이면 호스트 값, 아니면 platform과 같다(docs/notes/remote-2026-09-28.md 5.12). */
  hostPlatform: string;
  /** 다른 기기(원격 창·호스트 창)가 저장한 초안(같은 문서 6절 4). 바뀜 신호는 따로 오지 않는다. */
  onDraftChanged(listener: (sessionId: string, draft: Draft) => void): () => void;
  /** 다른 창·웹이 메모를 저장·삭제했을 때(저장한 연결에는 오지 않는다). */
  onNotesChanged(listener: (change: NoteChange) => void): () => void;
  /** 다른 창·웹이 보드·카드를 바꿨을 때(바꾼 연결에는 오지 않는다). */
  onBoardsChanged(listener: (change: BoardChange) => void): () => void;
  /** 앱 메뉴 명령(docs/notes/tab-shortcuts-2026-09-25.md). 창을 새로 열며 온 명령은 등록될 때 넘긴다. */
  onCommand(listener: (command: AppCommand) => void): () => void;
  /** 탭 닫기(⌘W)에 닫을 것이 없을 때 창을 닫는다. 창 닫기 단추와 같이 초안 저장 확인을 거친다(같은 문서 1번). */
  closeWindow(): void;
  /** 주 프로세스 몫 명령(백업·진단·종료 등)을 메뉴 누름과 같은 함수로 실행한다. 웹은 아무것도 하지 않는다(docs/notes/command-registry-2026-10-04.md 1a). */
  runMenuCommand(command: MainCommandId): void;
  /** 이 기기의 Lair를 끝낸다. 앱 메뉴 "Lair 종료"와 같은 확인 절차를 거친다(docs/notes/ui-redesign-2026-09-29.md 8절). 원격 창이면 이 기기 앱만 끝난다. */
  quitApp(): void;
  /** 화면이 첫 스냅숏을 그린 뒤 한 번 알린다(갱신 시험 기동의 건강 조건). 웹 화면은 없다. */
  rendererReady?(): void;
  /** 이 기기 앱의 판·빌드 번호·설치판 여부(같은 문서 8절). 빌드 번호는 포장할 때 앱 안에 적은 값이고 개발 실행은 dev다. 원격 창에서도 이 기기 값이다. */
  appInfo: { version: string; build: string; packaged: boolean };
  /** 늘린 가짜 언어로 화면을 그린다(개발 확인용, 다국어-8 H1). 설치판이 아니고 LAIR_NEXT_PSEUDO_LOCALE=1 로 띄웠을 때만 true. 웹에는 없다. */
  pseudoLocale?: boolean;
  /** 터미널 칸(docs/notes/terminal-pane-2026-09-27.md 3-1·3-2). 입력·출력은 보조 프로세스와 직통 통로로 오간다. */
  terminal: TerminalBridge;
  /** 미리보기 브라우저(docs/notes/preview-browser-2026-10-02.md D1~D3). 이 기기 데스크톱이 그린다. 웹 손님에는 없다. */
  browser?: PreviewBrowserBridge;
  /** 원격(docs/notes/remote-2026-09-28.md). 호스트 설정 "원격 연결"과 손님 모드. */
  remote: RemoteBridge;
  /** 앱 갱신(docs/notes/app-update-2026-10-03.md 화면과 원격). 이 앱의 갱신만 다룬다. 웹은 보기만이라 status 가 null 이고 동작은 거절한다. */
  update: {
    status(): Promise<UpdateStatus | null>;
    check(): Promise<UpdateStatus>;
    download(): Promise<UpdateStatus>;
    cancel(): Promise<UpdateStatus>;
    /** 다시 시작해 갱신. 확인 창·종료 확인은 주 프로세스가 한다. */
    apply(): Promise<void>;
    setAutoCheck(on: boolean): Promise<UpdateStatus>;
    /** 받은 새 판 폴더 열기(적용할 수 없는 판) */
    openFolder(): Promise<void>;
    onStatus(listener: (status: UpdateStatus) => void): () => void;
  };
  /** 이 기기의 앱 테마(docs/notes/ui-redesign-2026-09-29.md 6절). 호스트를 바꿔도 따라 바뀌지 않는다. */
  theme: UiThemeBridge;
  /** 창을 닫을 때 할 일(docs/notes/windows-close-2026-10-01.md 1절). 이 기기 설정이고 맥·웹에서는 null(고르는 칸이 없다). */
  closeBehavior: {
    get(): Promise<CloseBehavior | null>;
    set(mode: CloseMode): Promise<CloseBehavior | null>;
  };
  /** 이 컴퓨터 창의 앱 화면 배율(B-215). 웹 작업 화면에는 없다(브라우저 확대를 쓴다). */
  uiZoom?: UiZoomBridge;
}

export type { CloseBehavior, CloseMode, ConvColorDisplay, UiStyle, UiThemeBridge, UiThemePreference, UiThemeState } from './ui-theme-types';

// 원격 다리 타입(RemoteBridge)과 SSH 입력·결과 타입은 src/remote/shared.ts 로 옮겼다(이동만, 핫스팟 줄 수 확보).
export type { RemoteConnection, RemoteHostEntry };
/**
 * 호스트 폴더 고르기 권한의 목적(docs/notes/remote-2026-09-28.md 5.4). 권한은 목적이 맞을 때만 쓴다. 새 대화·옮기기로 고른 폴더는
 * 믿고 프로젝트로 등록하는 데도 쓴다(화면이 고른 뒤 등록을 먼저 부른다).
 */
/** 원격에서 부른 다시 시작·복원을 막는 것(폰 약속 14.2). 도는 터미널과 저장하지 않은 글은 resolve로 넘길 수 있다. */
export type RemoteBlocker = { kind: 'active-work'; count: number } | { kind: 'storage-error'; message: string } | { kind: 'copy-saving' } | { kind: 'send-pending' }
  | { kind: 'busy-terminals'; items: Array<{ terminalId: string; program: string }> }
  | { kind: 'unsaved'; items: Array<{ kind: 'draft' | 'rule-file' | 'common-rules' | 'edit-file' | 'setting'; name: string }> }
  | { kind: 'temporary'; count: number };
/** endTemporary 는 임시 보기 가능 연결만 넘길 수 있다(임시 대화는 다시 시작으로 끝난다, docs/notes/remote-system-2026-10-07.md D3). */
export interface RemoteResolve { closeTerminals?: boolean; discardUnsaved?: boolean; endTemporary?: boolean }

export type { PreviewTab, PreviewPick, PreviewConsoleEntry, PreviewCandidates, PreviewBounds, PreviewSurfaceState, PreviewBrowserWindowBridge } from './preview-tab';

export type { RemoteBrowserRect, RemoteBrowserInput, RemoteBrowserRequest, RemoteBrowserPickAt, RemoteBrowserFetched, RemoteBrowserCapture, RemoteBrowserRelay } from './preview-bridge';
/** 미리보기 브라우저 화면 통신(src/shared/preview-bridge.ts)에 스냅숏을 돌려주는 첨부만 더한 것. */
export interface PreviewBrowserBridge extends PreviewBrowserBridgeBase {
  /** 이미지를 그 대화 입력칸 첨부로 단다(클립보드를 거치지 않음, D4). tabId 면 그 탭의 보이는 화면을 찍고, dataUrl 이면 그 그림(고르기 크롭). */
  attachImage(sessionId: string, source: { tabId: string; name?: string; rect?: RemoteBrowserRect } | { dataUrl: string; name?: string }): Promise<Snapshot | null>;
}

declare global {
  interface Window { lair: LairBridge }
}
