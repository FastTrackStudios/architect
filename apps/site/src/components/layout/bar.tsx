import { Link } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { BrandMark } from "#/components/ui/brand-mark";
import { BAR, HERO_SHEET, SHEETS } from "#/content/landing";
import { NAV_LINKS } from "#/content/site";
import { PlotToggle } from "./plot-toggle";

/**
 * The bar across the top. On the landing page it carries the sheet index
 * (a disclosure on narrow screens; the sheet in view is marked while
 * scrolling); on the other pages, the site's own nav in its place.
 */
export function Bar({ sheets }: { sheets: boolean }) {
	return (
		<header className="bar" id="top">
			<Link className="brand" to="/" aria-label="Architect, home">
				<BrandMark />
				<span>Architect</span>
			</Link>
			{sheets ? (
				<SheetNav />
			) : (
				<nav className="bar-nav" aria-label="Site">
					<ul className="sheet-list site-list">
						{NAV_LINKS.map((l) => (
							<li key={l.to}>
								<Link to={l.to}>{l.label}</Link>
							</li>
						))}
					</ul>
				</nav>
			)}
			<div className="bar-end">
				{BAR.links.map((l) => (
					<a key={l.href} className="bar-link" href={l.href}>
						{l.label}
					</a>
				))}
				<PlotToggle />
			</div>
		</header>
	);
}

function SheetNav() {
	const nav = useRef<HTMLElement>(null);
	const button = useRef<HTMLButtonElement>(null);
	const [open, setOpen] = useState(false);
	const [current, setCurrent] = useState<{ id: string; label: string }>({
		id: "",
		label: HERO_SHEET,
	});

	// Escape closes it (focus back on the toggle); a click outside closes it.
	useEffect(() => {
		if (!open) return;
		const onKey = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				setOpen(false);
				button.current?.focus();
			}
		};
		const onClick = (e: MouseEvent) => {
			if (!nav.current?.contains(e.target as Node)) setOpen(false);
		};
		document.addEventListener("keydown", onKey);
		document.addEventListener("click", onClick);
		return () => {
			document.removeEventListener("keydown", onKey);
			document.removeEventListener("click", onClick);
		};
	}, [open]);

	// The sheet crossing the middle of the viewport is the current one.
	useEffect(() => {
		if (!("IntersectionObserver" in window)) return;
		const seen = new IntersectionObserver(
			(entries) => {
				for (const en of entries) {
					if (!en.isIntersecting) continue;
					const el = en.target as HTMLElement;
					setCurrent({ id: el.id, label: el.dataset.sheet ?? HERO_SHEET });
				}
			},
			{ rootMargin: "-45% 0px -50% 0px" },
		);
		for (const s of document.querySelectorAll(".sheet[data-sheet]"))
			seen.observe(s);
		return () => seen.disconnect();
	}, []);

	return (
		<nav
			ref={nav}
			className={open ? "bar-nav open" : "bar-nav"}
			aria-label="Sheets"
		>
			<button
				ref={button}
				className="bar-toggle"
				type="button"
				aria-expanded={open}
				aria-controls="sheet-list"
				onClick={() => setOpen(!open)}
			>
				<span className="bar-toggle-label">{BAR.sheetsLabel}</span>
				<span className="bar-current" data-current>
					{current.label}
				</span>
			</button>
			<ol className="sheet-list" id="sheet-list">
				{SHEETS.map((s) => (
					<li key={s.id}>
						<a
							href={`#${s.id}`}
							aria-current={current.id === s.id ? "true" : undefined}
							onClick={() => setOpen(false)}
						>
							<span className="sn">{s.sn}</span> {s.title}
						</a>
					</li>
				))}
			</ol>
		</nav>
	);
}
