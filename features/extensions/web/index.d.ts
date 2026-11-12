// @fasttrackstudio/architect: an Architect extension in any web app.
// docs/content/guide/extensions-in-your-web-app.md

/** How much an action may change (`manifest::Reach`). */
export type Effect = 'read_only' | 'mutating' | 'destructive';

/** What an extension may reach (an `extension.toml`'s `[capabilities]`, or what an embedder grants). */
export interface Capabilities {
  /** Action namespaces it may run (`THEME_`, `*`). */
  actions?: string[];
  /** The most an action it runs may change. */
  effect?: Effect;
  /** Network hosts it may fetch from (`api.example.com`, `*.example.com`, `host:port`, `*`). */
  hosts?: string[];
  folders?: string[];
  /** What of the app it may read: `org`, `selection`, `documents`, `editor`. */
  reads?: string[];
  /** What of the app it may change: `documents`, `editor`. */
  writes?: string[];
  /** The app's interface it may use: `notify`, `clipboard`, `pickers`, `prompts`, `menus`, `badges`. */
  ui?: string[];
  /** App events it may hear. */
  events?: string[];
  /** MIDI ports and OSC endpoints it may open. */
  ports?: string[];
  languages?: string[];
}

/** An `[[actions]]` entry of a manifest. */
export interface ActionDecl {
  id: string;
  name: string;
  description?: string;
  group?: string;
  shortcut?: string;
  effect?: Effect;
  /** Its arguments' JSON schema, as JSON text. */
  params?: string;
  [key: string]: unknown;
}

/** An extension's manifest (`extension.toml`), as `architect build-web` read it. */
export interface Manifest {
  id: string;
  name: string;
  version: string;
  kind?: string;
  /** The extension API it was written against. */
  api?: string;
  description?: string;
  capabilities?: Capabilities;
  actions?: ActionDecl[];
  views?: { id: string; title?: string; icon?: string; [key: string]: unknown }[];
  workers?: { id: string; title?: string; entry?: string; [key: string]: unknown }[];
  secrets?: { id: string; title?: string; hosts?: string[]; [key: string]: unknown }[];
  sections?: { settings?: { key: string; default?: unknown; [key: string]: unknown }[]; [key: string]: unknown }[];
  [key: string]: unknown;
}

/** What `architect-ext build-web` writes beside the transpiled module (`architect.json`). */
export interface Metadata {
  /** The transpiled module: a file inside its folder (relative; no scheme, no `..`). */
  module: string;
  /** The `architect:extension` world it was built for. */
  world: string;
  manifest: Manifest;
  /** A worker component's transpiled module, by worker id. */
  workers?: Record<string, string>;
}

/** Typed actions: each action id and what it takes and answers (`architect build-web` writes one per extension). */
export interface ActionMap {
  [id: string]: { args: unknown; result: unknown; effect?: Effect };
}

/** A loadExtension source given as its metadata: `folder` is the folder's URL. */
export interface MetadataSource extends Metadata {
  folder: string | URL;
}

/** `architect.lock.json`: each file's `sha256-<base64>`, by its path in the folder. */
export interface Lock {
  lockfileVersion: 1;
  files: Record<string, string>;
}

/** Untyped actions: any id, any arguments. */
export type AnyActions = Record<string, { args: unknown; result: unknown }>;

/** `run`'s arguments for action `K`: optional when it may be left out. */
export type RunArgs<A extends ActionMap, K extends keyof A> = undefined extends A[K]['args']
  ? [args?: A[K]['args']]
  : [args: A[K]['args']];

/** One of its actions. */
export interface ActionInfo {
  id: string;
  name: string;
  description: string;
  effect: Effect;
  /** Its arguments' JSON schema, or null when it takes none. */
  schema: unknown;
}

/** One of its views. */
export interface ViewInfo {
  id: string;
  title: string;
}

/** A line of its log. */
export interface LogLine {
  /** `debug`, `info`, `warn` or `error`. */
  level: string;
  /** Where it came from: `log`, `stdout`, `stderr`, `host`, `wasi`, `view`, a worker. */
  source: string;
  text: string;
}

/**
 * What one instance may use, and what keeps it from flooding the page (defaults in `LIMITS`). `memory`
 * is its wasm's: its own JS (jco's glue) in the sandbox frame uses what the browser gives a worker until
 * it is stopped (`stop()`, or `idle`).
 */
export interface Limits {
  /** Its memory, in bytes. */
  memory?: number;
  /** One call's own time, in ms. */
  call?: number;
  /** Log lines a second; past it they are dropped and counted. */
  logLines?: number;
  /** A log line's length, at most. */
  logLength?: number;
  /** Its views' shortest timer, in ms. */
  tick?: number;
  /** Restarts a minute (each after a longer wait); past it, it stays stopped. */
  restarts?: number;
  /** Its start (and its core modules' reading), at most, in ms. */
  start?: number;
  /** Core modules its JS may ask for. */
  modules?: number;
  /** A core module's size, at most, in bytes. */
  moduleBytes?: number;
  /** A host request's or a worker message's size, at most, in bytes. */
  messageBytes?: number;
  /** One `set_data` / `set_setting` value's size, at most, in bytes (as JSON). */
  dataBytes?: number;
  /** All its data and settings, at most, in bytes (as JSON). */
  dataTotal?: number;
  /**
   * After this many ms with no call, no view mounted and no worker running, it is stopped and its sandbox
   * frame removed (whatever its own JS was doing ends); its next call starts it again. 0: never.
   */
  idle?: number;
  /** Edits in one frame of a view. */
  editsPerFrame?: number;
  /** Nodes a mounted view may make. */
  nodesPerPane?: number;
  /** A mounted view's height, at most, in CSS px. */
  viewHeight?: number;
}

/** What it runs now. */
export interface Report {
  calls: number;
  traps: number;
  lastTrap: string | null;
  alive: boolean;
  /** How many times it started (a trap starts it again). */
  generation: number;
  memory: number;
  workers: { id: number; name: string; status: string; progress: unknown }[];
}

/** The app's theme as data, for its views. */
export interface Theme {
  id: string;
  scheme: string;
  revision: number;
  tokens: { name: string; value: string }[];
}

/** A host-service error (`hostcall::HostError`). */
export type HostError =
  | { kind: 'refused'; capability: string; why: string }
  | { kind: 'confirmation_required'; action: string; ticket: string }
  | { kind: 'not_loaded'; id: string }
  | { kind: 'not_found'; what: string }
  | { kind: 'failed'; message: string }
  | { kind: 'unsupported'; what: string };

/** A host-service error, thrown: `kind` and its fields; its message as the native host words it. */
export declare class ArchitectError extends Error {
  constructor(error: HostError);
  readonly kind: HostError['kind'];
  readonly capability?: string;
  readonly why?: string;
  readonly what?: string;
  readonly action?: string;
  readonly ticket?: string;
  toJSON(): HostError;
}


/** One edit of a document's text. */
export type DocEdit =
  | { insert: { at: number; text: string } }
  | { delete: { at: number; len: number } }
  | { append: { text: string } }
  | { replace: { text: string } };

/** A document, as an extension sees it. */
export interface DocInfo {
  path: string;
  name: string;
  space?: string;
}

/** Every request an extension makes of its host (`hostcall::HostRequest`), by `op`. */
export interface HostRequests {
  run: { action: string; args: unknown; ticket?: string | null };
  org: {};
  selection: {};
  open: { path: string };
  read_document: { path: string };
  list_documents: { under?: string | null };
  create_document: { path: string; text: string };
  edit_document: { path: string; edits: DocEdit[]; name?: string | null };
  rename_document: { path: string; to: string };
  move_document: { path: string; folder: string };
  delete_document: { path: string };
  watch_documents: { action?: string | null; script?: string | null; function?: string | null };
  editor_state: {};
  edit_editor: { changes?: unknown[]; selection?: unknown[] | null; name?: string | null; base?: number | null };
  decorate_editor: { decorations?: unknown[] };
  read_file: { path: string };
  write_file: { path: string; text: string };
  setting: { key: string };
  set_setting: { key: string; value: unknown };
  data: { key: string };
  set_data: { key: string; value?: unknown };
  log: { text: string; level?: string | null };
  service: { service: string; method: string; args: unknown };
  point: { point: string };
  window: { id: string; open?: boolean; state?: unknown };
  notify: { text: string; kind?: string | null; title?: string | null };
  clipboard_read: {};
  clipboard_write: { text: string };
  pick: { kind?: string; title?: string | null; filters?: unknown[]; folder?: string | null; name?: string | null };
  prompt: { kind?: string; title: string; message?: string | null; default?: string | null; placeholder?: string | null; choices?: unknown[] };
  menu: { items: unknown[]; x?: number | null; y?: number | null };
  decorate: { target: { kind: string; id: string }; decoration?: unknown };
  subscribe: { on: string; [key: string]: unknown };
  unsubscribe: { key: string };
  events: {};
  raise: { name: string; detail?: unknown };
  fetch: {
    url: string;
    method?: string | null;
    headers?: Record<string, string>;
    body?: string | null;
    timeout_ms?: number | null;
    max_bytes?: number | null;
    stream?: boolean;
    binary?: boolean;
  };
  read_body: { stream: string; max?: number | null };
  close_body: { stream: string };
  secret: { name: string };
  ask_secret: { name: string };
  ports: { kind?: string | null };
  open_port: { kind: string; name: string; listen?: number | null };
  port_send: { port: string; message: unknown };
  port_receive: { port: string; max?: number | null };
  close_port: { port: string };
  targets: {};
  surfaces: {};
  surface_bank: { surface: string; bank: number };
}

/** A host-service request's `op`. */
export type HostOp = keyof HostRequests;

/** Request `K`, as a handler gets it. */
export type HostRequest<K extends HostOp = HostOp> = { [P in K]: { op: P } & HostRequests[P] }[K];

/** Who asks: the extension's id, its manifest, and what it may reach (frozen). */
export interface HostContext {
  readonly extension: string;
  readonly manifest: Manifest;
  /**
   * Its capabilities within the grant. The package checked the request against them already; a handler
   * checks what only it can know: a path against `folders` (`read_file`, `write_file`), a stream, port or
   * hook being the extension's own, a service's or window's own rules.
   */
  readonly capabilities: Readonly<Required<Capabilities>>;
}

/**
 * Handlers for `fetch`, `read_body` and `close_body` that make an extension's requests the safe way:
 * `credentials: "omit"`, never to the page's own origin, only to `allow` (on top of its
 * `capabilities.hosts`), no redirect followed, a time and a size limit. Spread into `handle`.
 */
export declare function fetchHandler(options?: {
  allow?: string[];
  maxBytes?: number;
  timeoutMs?: number;
  /** Open streams per extension (default 8). */
  maxStreams?: number;
  /** Bytes its open streams may hold, per extension (default 32 MiB). */
  maxBuffered?: number;
  fetch?: typeof fetch;
}): Pick<HostHandlers, 'fetch' | 'read_body' | 'close_body'>;

/**
 * A handler for each `op` the embedder answers: what it returns is the answer; what it throws (an
 * `ArchitectError`, a `HostError`-shaped object, anything else as `failed`) is the error. An `op` with
 * no handler answers `unsupported` ("… is not available in this host"). Every request is checked against
 * the extension's capabilities within the grant BEFORE it reaches a handler.
 */
export type HostHandlers = {
  [K in HostOp]?: (request: HostRequest<K>, ctx: HostContext) => unknown | Promise<unknown>;
};

/** An app action the embedder offers extensions (`run-action`): its effect is what it is checked by. */
export interface AppAction {
  /** Default `destructive`: an action whose effect is not said needs the grant's `effect = "destructive"` and `confirm`. */
  effect?: Effect;
  run(args: unknown, ctx: HostContext): unknown | Promise<unknown>;
}

/** What an embedder implements: what the extension may ask the app for, and how it is answered. */
export interface Host {
  handle?: HostHandlers;
  /** App actions an extension may run (within its grant), by id. */
  actions?: Record<string, AppAction>;
  /**
   * Says yes or no to a destructive action: one of the extension's, run by the page, or an app action it
   * runs. Without it, none runs.
   */
  confirm?(action: { id: string; effect: Effect; by: string }, args: unknown): boolean | Promise<boolean>;
  /** Each line of its log. */
  log?(line: LogLine): void;
  /** The app's theme, for its views. */
  theme?(): Theme;
}

/** `memoryHost`'s options. */
export interface MemoryHostOptions {
  /** Its documents, by path. */
  documents?: Record<string, string>;
  /** Files it may read and change (only these), by path. */
  files?: Record<string, string>;
  org?: unknown;
  selection?: unknown;
  actions?: Record<string, AppAction>;
  confirm?: Host['confirm'];
  log?: Host['log'];
  theme?: Host['theme'];
  notify?(request: HostRequest<'notify'>): void;
  /** More handlers, or ones in place of its own. */
  handle?: HostHandlers;
}

/** A host in memory, good enough for a demo: nothing persisted, no network, no clipboard. */
export interface MemoryHost extends Host {
  handle: HostHandlers;
  documents: Map<string, string>;
  files: Map<string, string>;
  notifications: { text: string; kind: string | null; title: string | null }[];
}

export declare function memoryHost(options?: MemoryHostOptions): MemoryHost;

/** A Web Worker, or something shaped like one (`@fasttrackstudio/architect/node`'s `spawn`). */
export interface WorkerLike {
  onmessage: ((e: { data: unknown }) => void) | null;
  onerror: ((e: unknown) => void) | null;
  postMessage(message: unknown): void;
  terminate(): void;
  /** Under node: keep the process alive (while it starts or runs a call), or not. */
  ref?(): void;
  unref?(): void;
}

/** `loadExtension`'s options. */
export interface LoadOptions {
  /**
   * Pins its files: the `integrity` `build-web` printed (the lock's own `sha256-…`), or the lock itself;
   * each file's bytes are checked before use, a mismatch refuses it. Required: `false` loads its files
   * UNCHECKED (whoever serves them decides what runs in the sandbox), and says so in its log.
   */
  integrity: string | Lock | false;
  /** What answers its requests of the app. */
  host?: Host;
  /** What it may reach: its manifest's capabilities within this. Nothing by default. */
  grant?: Capabilities;
  /** Its instance's limits (default 64 MiB, 5000 ms a call). */
  limits?: Limits;
  /** Each worker's (default 64 MiB, 60000 ms a run). */
  workerLimits?: Limits;
  /** Its log, in place of `host.log`. */
  log?(line: LogLine): void;
  /** An instance's worker script, as source (the package's own by default). */
  instance?: string;
  /**
   * How an instance's worker is made: under node, `spawn` from `@fasttrackstudio/architect/node` (no
   * sandbox: node trusts the extension's JS). In a page, leave it out: the sandbox frame makes them.
   */
  spawn?(instance: string): WorkerLike;
  /** The sandbox document served by your own origin with its own CSP
   * (`sandbox allow-scripts; ` + `SANDBOX_CSP` + `; frame-ancestors 'self'`): the frame is
   * loaded from it instead of `srcdoc`, so your page's CSP needs no `SANDBOX_HASH`, `blob:` or
   * `'wasm-unsafe-eval'`. */
  sandbox?: string | URL;
}

/** A loaded extension. */
export declare class Extension<A extends ActionMap = AnyActions> {
  private constructor();
  readonly manifest: Manifest;
  /** The `architect:extension` world it was built for. */
  readonly world: string;
  /** What it may reach: its manifest's capabilities within the grant (frozen). */
  readonly capabilities: Readonly<Required<Capabilities>>;
  /** The actions it runs (each its manifest declares). */
  readonly actions: readonly ActionInfo[];
  /** The views it draws. */
  readonly views: readonly ViewInfo[];
  /** Runs action `id`: its answer. A destructive one runs only when the host's `confirm` says yes. */
  run<K extends keyof A & string>(id: K, ...args: RunArgs<A, K>): Promise<A[K]['result']>;
  report(): Report;
  /** Unloads it: its instance, workers, sandbox frame and views end. */
  stop(): void;
}

/**
 * Loads and starts the extension `architect build-web` wrote: `source` is its folder's URL, its
 * `architect.json`'s URL, or that metadata with `module` an absolute URL.
 */
export declare function loadExtension<A extends ActionMap = AnyActions>(
  source: string | URL | MetadataSource,
  options: LoadOptions,
): Promise<Extension<A>>;

/** A mounted view. */
export interface MountedView {
  id: number;
  /** Settles once its first frame is drawn. */
  rendered: Promise<void>;
  /** Sends a view mounted as an inline embed its input. */
  input(input: { claim: string; source: string; props?: unknown }): Promise<void>;
  unmount(): void;
}

/** `mountView`'s options. */
export interface MountOptions {
  /**
   * URL prefixes (absolute `https:` or `blob:`) its images and media may load from. None by default:
   * then only `data:` raster images; never a relative URL.
   */
  urls?: string[];
  /** A stylesheet for the view's shadow root, kept as `cleanSheet` keeps it (refused whole on
   * `@import`, fonts, escapes; `url()` only under `sheetUrls[0]`; `:host` keeps only custom
   * properties). */
  sheet?: string;
  /** The URL prefix its stylesheet's `url()`s may name (its first only). */
  sheetUrls?: string[];
}

/** Draws `extension`'s view `viewId` into `element` (emptied first), in a shadow root of its own. */
export declare function mountView(extension: Extension<any>, viewId: string, element: Element, options?: MountOptions): MountedView;

/** Whether this browser (or node) can run an extension: JS Promise Integration. Safari cannot yet. */
export declare function isSupported(): boolean;

/** The sandbox frame script's hash: an embedder's CSP `script-src` names it. */
export declare const SANDBOX_HASH: string;
/** The sandbox frame's own CSP. */
export declare const SANDBOX_CSP: string;
/** The lock file `build-web` writes. */
export declare const LOCK: 'architect.lock.json';

/** The `architect:extension` world this package runs. */
export declare const WORLD: string;
/** The manifest `api` it runs. */
export declare const API: string;
/** An instance's default limits. */
export declare const LIMITS: Required<Limits>;
/** A worker's default limits. */
export declare const WORKER_LIMITS: Required<Limits>;
/** The metadata file `architect build-web` writes. */
export declare const METADATA: 'architect.json';
