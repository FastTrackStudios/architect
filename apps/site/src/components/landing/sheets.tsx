import { Actions } from "#/components/ui/actions";
import { Arrow } from "#/components/ui/arrow";
import { Rich } from "#/components/ui/rich";
import {
	EXTENSIONS,
	FILES,
	FRAMEWORK,
	GET_ACTIONS,
	ISSUE,
	NOT_TO_SCALE,
	SELF_HOST,
	STUDIO,
	SYNC,
	VAULT,
} from "#/content/landing";
import { Detail } from "./detail";
import { Keynotes } from "./keynotes";
import { SectionCut } from "./section-cut";
import { Sheet, SheetHead } from "./sheet";

/** A-101: the vault. */
export function VaultSheet() {
	return (
		<Sheet
			id="vault"
			sheet="A-101 Vault"
			labelledBy="vault-title"
			strip={{ sn: "A-101", t: "Vault", s: NOT_TO_SCALE }}
		>
			<div className="sheet-body">
				<SheetHead id="vault-title" title={VAULT.title} lede={VAULT.lede} />
				<Detail
					detail={VAULT.tour}
					sheet="A-101"
					className="detail detail-wide"
				/>
				<Keynotes
					notes={VAULT.keynotes}
					label={VAULT.keynotesLabel}
					className="keynotes keynotes-grid"
				/>
				<div className="detail-pair">
					<Detail detail={VAULT.inbox} sheet="A-101" />
					<Detail
						detail={VAULT.phone}
						sheet="A-101"
						className="detail detail-phone"
					/>
				</div>
			</div>
		</Sheet>
	);
}

/** A-201: files. */
export function FilesSheet() {
	return (
		<Sheet
			id="files"
			sheet="A-201 Files"
			labelledBy="files-title"
			strip={{ sn: "A-201", t: "Files", s: NOT_TO_SCALE }}
		>
			<div className="sheet-body">
				<SheetHead id="files-title" title={FILES.title} lede={FILES.lede} />
				<SectionCut />
				<div className="files-grid">
					<Keynotes notes={FILES.keynotes} label={FILES.keynotesLabel} />
					<div className="detail-col">
						{FILES.details.map((d) => (
							<Detail key={d.n} detail={d} sheet="A-201" />
						))}
					</div>
				</div>
			</div>
		</Sheet>
	);
}

/** A-301: sync, as a schedule. */
export function SyncSheet() {
	return (
		<Sheet
			id="sync"
			sheet="A-301 Sync"
			labelledBy="sync-title"
			strip={{ sn: "A-301", t: "Sync", s: SYNC.strip }}
		>
			<div className="sheet-body">
				<SheetHead id="sync-title" title={SYNC.title} lede={SYNC.lede} />
				{/* biome-ignore lint/a11y: an ARIA table (div rows on a CSS grid), as the landing page drew it */}
				<div className="schedule" role="table" aria-label={SYNC.tableLabel}>
					{/* biome-ignore lint/a11y: as above */}
					<div className="sch-row sch-head" role="row">
						{SYNC.columns.map((c) => (
							// biome-ignore lint/a11y: as above
							<span key={c} role="columnheader">
								{c}
							</span>
						))}
					</div>
					{SYNC.rows.map((r) => (
						// biome-ignore lint/a11y: as above
						<div key={r.mark} className="sch-row" role="row">
							{/* biome-ignore lint/a11y: as above */}
							<span role="cell" className="sch-mark">
								{r.mark}
							</span>
							{/* biome-ignore lint/a11y: as above */}
							<span role="cell" className="sch-what">
								{r.what}
							</span>
							{/* biome-ignore lint/a11y: as above */}
							<span role="cell">
								<Rich text={r.how} />
							</span>
						</div>
					))}
				</div>
			</div>
		</Sheet>
	);
}

/** A-401: extensions. */
export function ExtensionsSheet() {
	return (
		<Sheet
			id="extensions"
			sheet="A-401 Extensions"
			labelledBy="ext-title"
			strip={{ sn: "A-401", t: "Extensions", s: NOT_TO_SCALE }}
		>
			<div className="sheet-body ext-body">
				<SheetHead
					id="ext-title"
					title={EXTENSIONS.title}
					lede={EXTENSIONS.lede}
				/>
				<Detail
					detail={EXTENSIONS.manager}
					sheet="A-401"
					className="detail detail-wide"
				/>
				<div className="ext-side">
					<Detail detail={EXTENSIONS.palette} sheet="A-401" />
				</div>
				<Keynotes
					notes={EXTENSIONS.keynotes}
					label={EXTENSIONS.keynotesLabel}
					className="keynotes ext-notes"
				/>
			</div>
		</Sheet>
	);
}

/** M-101: Studio. */
export function StudioSheet() {
	return (
		<Sheet
			id="studio"
			sheet="M-101 Studio"
			labelledBy="studio-title"
			className="sheet sheet-studio"
			strip={{ sn: "M-101", t: "Studio", s: NOT_TO_SCALE }}
		>
			<div className="sheet-body studio-body">
				<SheetHead id="studio-title" title={STUDIO.title} lede={STUDIO.lede} />
				<ul className="apps" aria-label={STUDIO.appsLabel}>
					{STUDIO.apps.map((a) => (
						<li key={a}>
							<span className="app-n">{a}</span>
						</li>
					))}
				</ul>
				<Detail
					detail={STUDIO.chart}
					sheet="M-101"
					className="detail detail-studio"
				/>
				<Keynotes
					notes={STUDIO.keynotes}
					label={STUDIO.keynotesLabel}
					className="keynotes studio-notes"
				/>
			</div>
		</Sheet>
	);
}

/** S-101: self-host. */
export function SelfHostSheet() {
	return (
		<Sheet
			id="self-host"
			sheet="S-101 Self-host"
			labelledBy="host-title"
			strip={{ sn: "S-101", t: "Self-host", s: SELF_HOST.strip }}
		>
			<div className="sheet-body host-body">
				<SheetHead
					id="host-title"
					title={SELF_HOST.title}
					lede={SELF_HOST.lede}
				/>
				<div className="notes-block">
					<h3>{SELF_HOST.notesTitle}</h3>
					{/* biome-ignore lint/a11y/noNoninteractiveTabindex: a code block that scrolls sideways */}
					<pre className="code" tabIndex={0}>
						<code>
							{SELF_HOST.code.map((l, i) => (
								// biome-ignore lint/suspicious/noArrayIndexKey: fixed lines, never reordered
								<span key={i}>
									{i > 0 ? "\n" : ""}
									{"comment" in l ? (
										<span className="c">{l.comment}</span>
									) : (
										l.line
									)}
								</span>
							))}
						</code>
					</pre>
					<dl className="spec">
						{SELF_HOST.spec.map((s) => (
							<div key={s.dt}>
								<dt>{s.dt}</dt>
								<dd>
									<Rich text={s.dd} />
								</dd>
							</div>
						))}
					</dl>
					<p className="notes-link">
						<a className="text-link" href={SELF_HOST.guide.href}>
							{SELF_HOST.guide.label}
						</a>
					</p>
				</div>
			</div>
		</Sheet>
	);
}

/** D-101: the framework. */
export function FrameworkSheet() {
	return (
		<Sheet
			id="framework"
			sheet="D-101 Framework"
			labelledBy="dev-title"
			className="sheet sheet-dev"
			strip={{ sn: "D-101", t: "Framework", s: FRAMEWORK.strip }}
		>
			<div className="sheet-body dev-body">
				<SheetHead
					id="dev-title"
					title={FRAMEWORK.title}
					lede={FRAMEWORK.lede}
				/>
				<figure className="code-detail">
					{/* biome-ignore lint/a11y/noNoninteractiveTabindex: a code block that scrolls sideways */}
					<pre className="code" tabIndex={0}>
						<code>
							<span className="k">#[architect::service(actions)]</span>
							{"\n"}
							<span className="k">pub trait</span> Greeter {"{"}
							{"\n    "}
							<span className="k">async fn</span> greet(&amp;self, name: String)
							-&gt; Result&lt;String, GreetError&gt;;
							{"\n    "}
							<span className="k">#[subscribe]</span>
							{"\n    "}
							<span className="k">fn</span> greeted(&amp;self) -&gt; String;
							{"\n}"}
						</code>
					</pre>
					<figcaption>
						<Rich text={FRAMEWORK.codeCaption} />
					</figcaption>
				</figure>
				<Detail
					detail={FRAMEWORK.views}
					sheet="D-101"
					className="detail detail-dev"
				/>
				<Keynotes
					notes={FRAMEWORK.keynotes}
					label={FRAMEWORK.keynotesLabel}
					className="keynotes keynotes-grid dev-notes"
				/>
				<p className="dev-cta">
					<a className="act act-inline" href={FRAMEWORK.docs.href}>
						<span className="act-k">{FRAMEWORK.docs.label}</span>
						<Arrow dir="right" />
					</a>{" "}
					<a className="text-link" href={FRAMEWORK.source.href}>
						{FRAMEWORK.source.label}
					</a>
				</p>
			</div>
		</Sheet>
	);
}

/** Issued for use: the closing call. */
export function IssueSheet() {
	return (
		<Sheet
			id="get"
			sheet="Issue"
			labelledBy="get-title"
			className="sheet sheet-issue"
		>
			<div className="issue">
				<h2 id="get-title">{ISSUE.title}</h2>
				<Actions
					actions={GET_ACTIONS}
					label="Get Architect"
					className="tb-actions issue-actions"
				/>
				<div className="issue-read">
					{ISSUE.read.map((r) => (
						<a key={r.href} className="text-link" href={r.href}>
							{r.label}
						</a>
					))}
				</div>
			</div>
		</Sheet>
	);
}
