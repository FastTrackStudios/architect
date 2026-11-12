import { Link } from "@tanstack/react-router";

import { AppleIcon, LinuxIcon } from "#/components/ui/icons";
import { Shot } from "#/components/ui/shot";
import { HERO } from "#/content/landing";
import { RELEASE } from "#/lib/data";

/**
 * The first viewport: what it is, the download, and the app itself. The
 * tabs over the screenshot are radios and their labels, so they switch it
 * with no script at all (styles/landing.css, `:has(:checked)`).
 */
export function Hero() {
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
				{RELEASE.linux ? (
					<a className="btn os-linux" href={RELEASE.linux.url}>
						<LinuxIcon />
						{HERO.download}
						<span className="vh">
							{" "}
							{HERO.for.replace("{platform}", HERO.linux)}
						</span>
					</a>
				) : null}
				{RELEASE.macos ? (
					<a className="btn os-macos" href={RELEASE.macos.url}>
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
			<div className="viewer">
				<div className="tabs" role="radiogroup" aria-label={HERO.pick}>
					{HERO.shots.map((s, i) => (
						<label key={s.id} className="tab">
							<input
								className="vh"
								type="radio"
								name="hero-shot"
								id={`pick-${s.id}`}
								defaultChecked={i === 0}
							/>
							{s.label}
						</label>
					))}
				</div>
				<div className="frame viewer-shots">
					{HERO.shots.map((s, i) => (
						<div key={s.id} className={`viewer-shot viewer-${s.id}`}>
							<Shot media={s.media} eager={i === 0} />
						</div>
					))}
				</div>
			</div>
		</section>
	);
}
