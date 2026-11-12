import { Link } from "@tanstack/react-router";

import { BrandMark } from "#/components/ui/brand-mark";
import { NAV_LINKS, SHELL, SITE } from "#/content/site";
import { ThemeToggle } from "./theme-toggle";

/** The header, the same on every page: the mark, the nav, the theme. */
export function SiteHeader() {
	return (
		<header className="site-header wrap">
			<Link className="brand" to="/" aria-label={SHELL.home}>
				<BrandMark />
				<span>{SITE.name}</span>
			</Link>
			<nav className="site-nav" aria-label={SHELL.nav}>
				<ul>
					{NAV_LINKS.map((l) => (
						<li key={l.to}>
							<Link to={l.to} activeProps={{ "aria-current": "page" }}>
								{l.label}
							</Link>
						</li>
					))}
					<li>
						<a href={SHELL.docs.href}>{SHELL.docs.label}</a>
					</li>
				</ul>
			</nav>
			<ThemeToggle />
		</header>
	);
}
