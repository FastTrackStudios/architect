/**
 * The landing page's words (#559). The page is not screenshot-driven: it
 * says what Architect is in six short entries, then how to get it.
 *
 * Where something named here is not shipped yet, its chip or its entry
 * carries a `note` ("coming", "views today") and the issue that tracks it,
 * so the page never claims more than the code does.
 */
import { DOCS_PATH, SITE } from "./site";

const issue = (n: number) => `${SITE.repo}/issues/${n}`;

export const HERO = {
	title: "Build the systems that work for you.",
	sub: "Architect is a modular, local-first, collaborative foundation.",
	/** One button: the visitor's own platform, shown by its icon. */
	download: "Download",
	for: "for {platform}",
	linux: "Linux",
	macos: "macOS",
	more: { label: "More platforms", to: "/download" },
	demo: { label: "Demo", href: "/app/?demo" },
};

export type Chip = { name: string; note?: string; href?: string };

export type Pillar = {
	id: string;
	/** Which drawing acts it out (src/components/landing/figures.tsx). */
	figure:
		| "modular"
		| "local-first"
		| "collaborative"
		| "extensions"
		| "platforms"
		| "embeddable";
	title: string;
	body: string;
	/** A status on the whole entry, with where it is tracked. */
	status?: { label: string; href: string };
	chips?: { label: string; items: readonly Chip[] };
	code?: { lines: readonly string[] };
	link?: { label: string; href?: string; to?: string };
};

export const PILLARS: readonly Pillar[] = [
	{
		id: "modular",
		figure: "modular",
		title: "Modular",
		body: "Every feature is an extension. Keep the ones you use, replace the ones you don't.",
	},
	{
		id: "local-first",
		figure: "local-first",
		title: "Local-first",
		body: "Your work is a folder of markdown on your own disk. No account, no network needed.",
	},
	{
		id: "collaborative",
		figure: "collaborative",
		title: "Collaborative",
		body: "Live editing and shared cursors, device to device.",
		status: { label: "Coming to the app", href: issue(383) },
	},
	{
		id: "extensions",
		figure: "extensions",
		title: "Extensions",
		body: "A sandboxed, permissioned extension system, in the language you already write.",
		chips: {
			label: "Write an extension in",
			items: [
				{ name: "Rust" },
				// Views are TSX today; actions in TypeScript are #536.
				{ name: "TypeScript", note: "views today", href: issue(536) },
				{ name: "Lua" },
				{ name: "Anything that compiles to WASM" },
			],
		},
		link: {
			label: "How extensions work",
			href: `${DOCS_PATH}architecture/extensions/`,
		},
	},
	{
		id: "platforms",
		figure: "platforms",
		title: "Every platform",
		body: "One app, the same workspace everywhere.",
		chips: {
			label: "Runs on",
			items: [
				{ name: "Linux" },
				{ name: "macOS" },
				{ name: "Web", note: "preview", href: issue(383) },
				{ name: "Mobile", note: "coming", href: issue(383) },
				{ name: "Windows", note: "coming", href: issue(431) },
			],
		},
	},
	{
		id: "embeddable",
		figure: "embeddable",
		title: "Embeddable",
		body: "Run an Architect extension inside your own web app, or ship your own Architect.",
		status: { label: "npm package coming", href: issue(535) },
		code: {
			lines: [
				"import { loadExtension, mountView } from '@fasttrackstudio/architect';",
				"",
				"const ext = await loadExtension('/counter/', { integrity, host });",
				"await mountView(ext, 'counter', element).rendered;",
			],
		},
		link: {
			label: "Extensions in your web app",
			href: `${DOCS_PATH}guide/extensions-in-your-web-app/`,
		},
	},
];

export const GET = {
	id: "get",
	title: "Get Architect.",
	linux: { k: "Linux", v: "AppImage" },
	macos: { k: "macOS", v: "Apple silicon" },
	nix: { k: "Nix" },
	web: { k: "Web", href: "/app/", open: "Open the web app" },
	download: "Download",
	missing: "Not in this release",
	copy: "Copy",
	copied: "Copied",
	version: "v{version}",
	more: [
		{ label: "Docs", href: DOCS_PATH },
		{ label: "Extensions", to: "/extensions" },
		{ label: "Changelog", to: "/changelog" },
		{ label: "Source", href: SITE.repo },
	] as ReadonlyArray<{ label: string; href?: string; to?: string }>,
};
