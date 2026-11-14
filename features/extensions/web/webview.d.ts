// The web view kind's SDK (#222): what a web view app sees of Architect.
//
// `import { architect } from '@fasttrackstudio/architect/webview'` in a
// Vite/TypeScript app shipped as an extension's `kind = "web"` view. The
// host puts the runtime (`window.architect`) in the view before its own
// scripts; this module gives it types. Every call is a host-service request
// (`features/extensions/src/hostcall.rs` `HostRequest`): the host checks it
// against the extension's capabilities as the person granted them, and a
// refused call rejects with `kind: "refused"`. `HostRequest` below names every
// op the host has (a test in `features/extensions/src/webview.rs` keeps it in
// step).

/** A JSON value. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** A document, as the app lists it. */
export interface DocInfo {
  path: string;
  name: string;
  /** The space it is in, when the app has more than one. */
  space?: string;
}

/** One edit of a document's text (character offsets); a call's edits are one undo step. */
export type DocEdit =
  | { insert: { at: number; text: string } }
  | { delete: { at: number; len: number } }
  | { append: { text: string } }
  | { replace: { text: string } };

/** A file filter of a picker. */
export interface FileFilter {
  name: string;
  extensions: string[];
}

/** One item of a menu. */
export interface MenuItem {
  id: string;
  label: string;
  icon?: string;
  shortcut?: string;
  disabled?: boolean;
  separator?: boolean;
}

/** One choice of a `pick_one` prompt. */
export interface Choice {
  id: string;
  label: string;
  detail?: string;
}

/** An HTTP request through the host: only to the hosts in the manifest's `capabilities.hosts`. */
export interface Fetch {
  url: string;
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  timeout_ms?: number;
  max_bytes?: number;
  stream?: boolean;
  binary?: boolean;
}

/** Its answer. A header's value can name a secret, `{{secret:<name>}}`: the host puts it in. */
export interface FetchAnswer {
  status: number;
  url: string;
  headers: Record<string, string>;
  /** Text, or base64 when `binary`. */
  body: string;
  /** With `stream`: read the body with `read_body`. */
  stream?: string;
}

/** Every host-service request. */
export type HostRequest =
  | { op: "run"; action: string; args?: Json; ticket?: string | null }
  | { op: "org" }
  | { op: "selection" }
  | { op: "open"; path: string }
  | { op: "read_document"; path: string }
  | { op: "list_documents"; under?: string | null }
  | { op: "create_document"; path: string; text?: string }
  | { op: "edit_document"; path: string; edits: DocEdit[]; name?: string | null }
  | { op: "rename_document"; path: string; to: string }
  | { op: "move_document"; path: string; folder: string }
  | { op: "delete_document"; path: string }
  | { op: "watch_documents"; action?: string; script?: string; function?: string }
  | { op: "editor_state" }
  | { op: "edit_editor"; changes?: Json[]; selection?: Json[]; name?: string; base?: number }
  | { op: "decorate_editor"; decorations?: Json[] }
  | { op: "read_file"; path: string }
  | { op: "write_file"; path: string; text: string }
  | { op: "setting"; key: string }
  | { op: "set_setting"; key: string; value: Json }
  | { op: "data"; key: string }
  | { op: "set_data"; key: string; value?: Json }
  | { op: "log"; text: string; level?: "debug" | "info" | "warn" | "error" | null }
  | { op: "service"; service: string; method: string; args?: Json }
  | { op: "point"; point: string }
  | { op: "window"; id: string; open: boolean; state?: Json }
  | { op: "notify"; text: string; kind?: "info" | "success" | "warning" | "error"; title?: string }
  | { op: "clipboard_read" }
  | { op: "clipboard_write"; text: string }
  | { op: "pick"; kind: "open" | "open_many" | "folder" | "save"; title?: string; filters?: FileFilter[]; folder?: string; name?: string }
  | { op: "prompt"; kind: "text" | "confirm" | "pick_one" | "secret"; title: string; message?: string; default?: string; placeholder?: string; choices?: Choice[] }
  | { op: "menu"; items: MenuItem[]; x?: number; y?: number }
  | { op: "decorate"; target: { kind: string; id: string }; decoration?: Json }
  | { op: "subscribe"; on?: string; every?: string; at?: string; action?: string; script?: string; function?: string }
  | { op: "unsubscribe"; key: string }
  | { op: "events" }
  | { op: "raise"; name: string; detail?: Json }
  | ({ op: "fetch" } & Fetch)
  | { op: "read_body"; stream: string; max?: number }
  | { op: "close_body"; stream: string }
  | { op: "secret"; name: string }
  | { op: "ask_secret"; name: string }
  | { op: "store_secret"; name: string; value: string }
  | { op: "ports"; kind?: string }
  | { op: "open_port"; kind: "midi" | "osc"; name: string; listen?: number }
  | { op: "port_send"; port: string; message: Json }
  | { op: "port_receive"; port: string; max?: number }
  | { op: "close_port"; port: string }
  | { op: "targets" }
  | { op: "surfaces" }
  | { op: "surface_bank"; surface: string; bank: number };

/** Why a call did not do what it asked (the rejection's `kind`). */
export type HostErrorKind =
  | "refused"
  | "confirmation_required"
  | "not_loaded"
  | "not_found"
  | "failed"
  | "unsupported";

export interface HostError extends Error {
  kind: HostErrorKind;
  detail: { kind: HostErrorKind; [key: string]: Json };
}

/** `window.architect`: the bridge to the app. */
export interface Architect {
  /** Any host-service request: its answer, or a rejected {@link HostError}. */
  call<T = Json>(request: HostRequest): Promise<T>;
  /** Runs an app action as the extension (its own, or within `capabilities.actions`). */
  run<T = Json>(action: string, args?: Json): Promise<T>;
  /** One of its own settings (`<id>.…`). */
  setting<T = Json>(key: string): Promise<T>;
  setSetting(key: string, value: Json): Promise<Json>;
  /** Its own data, kept for it. */
  data<T = Json>(key: string): Promise<T | null>;
  setData(key: string, value?: Json): Promise<Json>;
  /** The app's documents (`reads`/`writes = ["documents"]`). */
  documents: {
    list(under?: string): Promise<DocInfo[]>;
    read(path: string): Promise<string>;
    create(path: string, text?: string): Promise<DocInfo>;
    edit(path: string, edits: DocEdit[], name?: string): Promise<string>;
    rename(path: string, to: string): Promise<DocInfo>;
    move(path: string, folder: string): Promise<DocInfo>;
    remove(path: string): Promise<Json>;
    open(path: string): Promise<Json>;
  };
  /** HTTP through the host, to `capabilities.hosts` only (the view itself has no network). */
  fetch(url: string, options?: Omit<Fetch, "url">): Promise<FetchAnswer>;
  /** Its `[[secrets]]`: whether one is set, ask the person, or keep one (insert-only). */
  secrets: {
    isSet(name: string): Promise<{ set: boolean; title: string | null }>;
    ask(name: string): Promise<boolean>;
    store(name: string, value: string): Promise<boolean>;
  };
  notify(text: string, options?: { kind?: "info" | "success" | "warning" | "error"; title?: string }): Promise<Json>;
  prompt(options: Omit<Extract<HostRequest, { op: "prompt" }>, "op">): Promise<string | boolean | null>;
  pick(options: Omit<Extract<HostRequest, { op: "pick" }>, "op">): Promise<string | string[] | null>;
  menu(items: MenuItem[], at?: { x: number; y: number }): Promise<string | null>;
  clipboard: { read(): Promise<string | null>; write(text: string): Promise<Json> };
  log(text: string, level?: "debug" | "info" | "warn" | "error"): Promise<Json>;
  /** Hears `doc.saved`, `doc.*`, `app:<name>`… while it may: returns a function that stops it. */
  on(event: string, listener: (detail: Json, name: string) => void): () => void;
  /** The app's theme: its tokens are CSS custom properties on `:root` (`var(--brand)`). */
  theme: { readonly scheme: "light" | "dark"; onChange(listener: (scheme: "light" | "dark") => void): () => void };
  /** Whether the view is shown (its tab in front, nothing of the app's over it). */
  readonly shown: boolean;
  onShown(listener: (shown: boolean) => void): () => void;
}

/** The bridge; `undefined` outside an Architect web view (a dev server in a browser tab). */
export declare const architect: Architect | undefined;

/** The bridge, or an error saying the page is not in an Architect web view. */
export declare function requireArchitect(): Architect;
