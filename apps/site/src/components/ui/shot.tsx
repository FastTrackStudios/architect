import { useEffect, useRef, useState } from "react";

import type { Media } from "#/content/landing";

/**
 * A real capture, an image or a looping clip. Images are sized (no layout
 * shift) and lazy unless `eager`; `sizes` says how wide the slot is so a
 * phone takes the narrower copy.
 */
export function Shot({
	media: m,
	eager,
	sizes = "(max-width: 72rem) 100vw, 68rem",
}: {
	media: Media;
	eager?: boolean;
	sizes?: string;
}) {
	if (m.kind === "video") return <LoopVideo {...m} />;
	return (
		<img
			src={m.src}
			srcSet={
				m.small
					? `${m.small.src} ${m.small.width}w, ${m.src} ${m.width}w`
					: undefined
			}
			sizes={m.small ? sizes : undefined}
			width={m.width}
			height={m.height}
			loading={eager ? "eager" : "lazy"}
			fetchPriority={eager ? "high" : undefined}
			decoding="async"
			alt={m.alt}
		/>
	);
}

/**
 * A looping clip: plays while a third of it is on screen, pauses off
 * screen, and never starts by itself under reduced motion. Play/Pause is
 * the reader's: a clip they paused stays paused, one they played plays
 * even under reduced motion. With scripts off it is its poster.
 */
function LoopVideo({
	src,
	poster,
	width,
	height,
	alt,
}: {
	src: string;
	poster: string;
	width: number;
	height: number;
	alt: string;
}) {
	const video = useRef<HTMLVideoElement>(null);
	const user = useRef<"" | "play" | "pause">("");
	const [paused, setPaused] = useState(true);

	useEffect(() => {
		const v = video.current;
		if (!v || !("IntersectionObserver" in window)) return;
		const reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
		const seen = new IntersectionObserver(
			(entries) => {
				for (const en of entries) {
					if (en.isIntersecting) {
						if (
							user.current === "pause" ||
							(reduce.matches && user.current !== "play")
						)
							return;
						v.preload = "auto";
						v.play().catch(() => {});
					} else if (!v.paused) {
						v.pause();
					}
				}
			},
			{ threshold: 0.35 },
		);
		seen.observe(v);
		return () => seen.disconnect();
	}, []);

	return (
		<div className="clip">
			<video
				ref={video}
				muted
				loop
				playsInline
				preload="none"
				poster={poster}
				width={width}
				height={height}
				aria-label={alt}
				onPlay={() => setPaused(false)}
				onPause={() => setPaused(true)}
			>
				<source src={src} type="video/mp4" />
			</video>
			<button
				className="clip-btn"
				type="button"
				aria-pressed={!paused}
				onClick={() => {
					const v = video.current;
					if (!v) return;
					if (v.paused) {
						user.current = "play";
						v.play().catch(() => {});
					} else {
						user.current = "pause";
						v.pause();
					}
				}}
			>
				{paused ? "Play" : "Pause"}
			</button>
		</div>
	);
}
