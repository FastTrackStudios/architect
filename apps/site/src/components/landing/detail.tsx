import { useEffect, useRef, useState } from "react";

import type { Detail as DetailT } from "#/content/landing";

/**
 * A detail: a screenshot or a looping clip in a frame, numbered with its
 * sheet's bubble.
 */
export function Detail({
	detail,
	sheet,
	className = "detail",
}: {
	detail: DetailT;
	sheet: string;
	className?: string;
}) {
	const m = detail.media;
	return (
		<figure className={className}>
			<div className="detail-frame">
				{m.kind === "video" ? (
					<LoopVideo {...m} />
				) : (
					<img
						className="shot"
						src={m.src}
						width={m.width}
						height={m.height}
						loading="lazy"
						decoding="async"
						alt={m.alt}
					/>
				)}
			</div>
			<figcaption>
				<span className="bubble">
					{detail.n}
					<i>{sheet}</i>
				</span>{" "}
				{detail.caption}
			</figcaption>
		</figure>
	);
}

/**
 * A looping clip: plays while a third of it is on screen, pauses off
 * screen, and never starts by itself under reduced motion. Play/Pause is
 * the reader's: a clip they paused stays paused, one they played plays
 * even under reduced motion.
 */
function LoopVideo({
	src,
	poster,
	width,
	height,
	label,
}: {
	src: string;
	poster: string;
	width: number;
	height: number;
	label: string;
}) {
	const video = useRef<HTMLVideoElement>(null);
	const user = useRef<"" | "play" | "pause">("");
	const [paused, setPaused] = useState(true);

	useEffect(() => {
		const v = video.current;
		if (!v || !("IntersectionObserver" in window)) return;
		const reduce = window.matchMedia("(prefers-reduced-motion: reduce)");
		const vo = new IntersectionObserver(
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
		vo.observe(v);
		return () => vo.disconnect();
	}, []);

	return (
		<>
			<video
				ref={video}
				className="shot"
				data-loop
				muted
				loop
				playsInline
				preload="none"
				poster={poster}
				width={width}
				height={height}
				aria-label={label}
				onPlay={() => setPaused(false)}
				onPause={() => setPaused(true)}
			>
				<source src={src} type="video/mp4" />
			</video>
			<button
				className="loop-btn"
				type="button"
				data-loop-btn
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
		</>
	);
}
