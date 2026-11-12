import { useEffect, useState } from "react";

/**
 * One command line to copy. The line is in the HTML; the button appears
 * with scripts on (`.js .cmd .copy`) and says what it did.
 */
export function Cmd({
	text,
	copy,
	copied,
}: {
	text: string;
	copy: string;
	copied: string;
}) {
	const [done, setDone] = useState(false);
	useEffect(() => {
		if (!done) return;
		const t = setTimeout(() => setDone(false), 2000);
		return () => clearTimeout(t);
	}, [done]);
	return (
		<div className="cmd">
			<code>{text}</code>
			<button
				className="copy"
				type="button"
				onClick={() => {
					navigator.clipboard?.writeText(text).then(
						() => setDone(true),
						() => {},
					);
				}}
			>
				<span aria-live="polite">{done ? copied : copy}</span>
			</button>
		</div>
	);
}
