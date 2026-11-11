import type { HTMLAttributes, ReactElement } from 'react';
import type { Extension } from './index.js';

export interface ArchitectViewProps extends HTMLAttributes<HTMLDivElement> {
  /** A loaded extension (`loadExtension`); nothing is drawn until it is set. */
  extension: Extension<any> | null | undefined;
  /** Which of its views. */
  view: string;
  /** What a view mounted as an inline embed draws: `{ claim, source, props }`. */
  input?: { claim: string; source: string; props?: unknown };
}

/** An extension's view, drawn into a `div` while it is mounted. */
export declare function ArchitectView(props: ArchitectViewProps): ReactElement;
