// @fasttrackstudio/architect/react: an extension's view as a React
// component, `mountView` under a `div` it owns.
import { createElement, useEffect, useRef } from 'react';
import { mountView } from './index.js';

/// `<ArchitectView extension={ext} view="counter" />`: draws `view` while
/// mounted; `input` is what a view mounted as an inline embed draws.
export function ArchitectView({ extension, view, input, ...props }) {
  const ref = useRef(null);
  const mounted = useRef(null);
  useEffect(() => {
    if (!extension || !ref.current) return undefined;
    const m = mountView(extension, view, ref.current);
    mounted.current = m;
    return () => {
      mounted.current = null;
      m.unmount();
    };
  }, [extension, view]);
  useEffect(() => {
    const m = mounted.current;
    if (m && input !== undefined) m.rendered.then(() => mounted.current === m && m.input(input));
  }, [extension, view, input]);
  return createElement('div', { ...props, ref });
}
