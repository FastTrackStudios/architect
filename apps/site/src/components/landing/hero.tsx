import { Link } from "@tanstack/react-router";

import { AppleIcon, LinuxIcon } from "#/components/ui/icons";
import { HERO } from "#/content/landing";
import { RELEASE } from "#/lib/data";
import { useReleaseAssets } from "#/lib/release-check";

/**
 * The first viewport: what it is, and the three things to do about it.
 */
export function Hero() {
	// Named by convention from the tag; a build GitHub does not list (yet)
	// loses its button and the platform's visitors get /download's.
	const { assets } = useReleaseAssets(RELEASE.tag);
	const { linux, macos } = assets;
	return (
		<section className="hero wrap" aria-labelledby="hero-title">
			<h1 id="hero-title">{HERO.title}</h1>
			<p className="sub">{HERO.sub}</p>
			<p className="hero-get">
				{/* One Download button. The head script names the platform
				    (<html data-os>) and CSS shows that one; with no script, or on
				    a platform with no build, the button goes to /download. */}
				<Link className="btn os-any" to={HERO.more.to}>
					{HERO.download}
				</Link>
				{linux.state !== "missing" ? (
					<a className="btn os-linux" href={linux.url}>
						<LinuxIcon />
						{HERO.download}
						<span className="vh">
							{" "}
							{HERO.for.replace("{platform}", HERO.linux)}
						</span>
					</a>
				) : null}
				{macos.state !== "missing" ? (
					<a className="btn os-macos" href={macos.url}>
						<AppleIcon />
						{HERO.download}
						<span className="vh">
							{" "}
							{HERO.for.replace("{platform}", HERO.macos)}
						</span>
					</a>
				) : null}
				<a className="btn btn-quiet" href={HERO.demo.href}>
					{HERO.demo.label}
				</a>
				<Link className="hero-more" to={HERO.more.to}>
					{HERO.more.label}
				</Link>
			</p>
		</section>
	);
}
