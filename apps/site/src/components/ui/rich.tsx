import { Fragment, type ReactNode } from "react";

/**
 * The inline markup src/content uses, and nothing more: `**bold**`,
 * `` `code` `` and `[text](href)`. Content stays plain strings a redesign
 * can restyle; this is the one place they become elements.
 */
const TOKEN = /\*\*(.+?)\*\*|`([^`]+)`|\[([^\]]+)\]\(([^)]+)\)/g;

export function Rich({ text }: { text: string }) {
	const out: ReactNode[] = [];
	let last = 0;
	for (const m of text.matchAll(TOKEN)) {
		const at = m.index ?? 0;
		if (at > last) out.push(text.slice(last, at));
		if (m[1] !== undefined) out.push(<strong key={at}>{m[1]}</strong>);
		else if (m[2] !== undefined) out.push(<code key={at}>{m[2]}</code>);
		else
			out.push(
				<a key={at} href={m[4]}>
					{m[3]}
				</a>,
			);
		last = at + m[0].length;
	}
	if (last < text.length) out.push(text.slice(last));
	return <Fragment>{out}</Fragment>;
}
