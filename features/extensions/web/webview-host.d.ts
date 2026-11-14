// The web view kind's browser host (#222): see webview-host.js.
import type { HostRequest, Json } from './webview.js';

/** The policy a web view's files are served under. */
export declare const WEBVIEW_CSP: string;
/** The largest message a view may send. */
export declare const MAX_MESSAGE: number;

export interface WebViewTheme {
  scheme: 'light' | 'dark';
  /** Token name to CSS value (`brand` → `#6c5ce7`). */
  tokens: Record<string, string>;
}

export type HostAnswer = { ok: Json } | { error: { kind: string; [key: string]: Json } };

export interface WebViewOptions {
  /** Its document's URL: its package's file, served under `WEBVIEW_CSP`. */
  src: string;
  /** Answers one host-service request, checked against what the extension may reach. */
  call(request: HostRequest): Promise<HostAnswer> | HostAnswer;
  theme?: WebViewTheme;
  /** An Escape the view did not take: give the focus back to the page. */
  onEscape?(): void;
  /** The frame's accessible name. */
  title?: string;
}

export interface MountedWebView {
  frame: HTMLIFrameElement;
  setTheme(theme: WebViewTheme): void;
  setShown(shown: boolean): void;
  /** Passes an event on when the view listens for it. */
  emit(name: string, detail: Json): void;
  unmount(): void;
}

/** The theme as a style sheet: each token a custom property on `:root`. */
export declare function themeCss(scheme: 'light' | 'dark', tokens: Record<string, string>): string;

/** Mounts a web view in `container`: a sandboxed `<iframe>` (`allow-scripts` only). */
export declare function mountWebView(container: HTMLElement, options: WebViewOptions): MountedWebView;
