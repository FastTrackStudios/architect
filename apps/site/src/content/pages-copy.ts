/**
 * The words of the pages beside the landing page: /download, /changelog,
 * /extensions and the 404. What they list comes from src/data (written at
 * build time by scripts/data.ts) and, for /download's files, GitHub's API
 * in the browser (src/lib/release-assets.ts); what they say is here.
 */
import { DOCS_PATH, NIX_RUN, ORG_RELEASE_KEY, SITE } from "./site";

export const DOWNLOAD = {
	title: "Download",
	description:
		"Architect for Linux and macOS, on Nix, and the node for a home server: the latest release's downloads.",
	lede: "The latest release of the Architect app. Each download is checked by the app's updater against its sha256, and signed in SHA256SUMS with the org key.",
	release: "Release",
	get: "Download",
	web: {
		k: "Web app",
		v: "In your browser",
		href: "/app/",
		open: "Open",
		note: "Sign-in and your node's status today. Notes, projects and files come to the browser as they are ported.",
	},
	updates:
		"On Linux the app updates itself from here, after checking the signed SHA256SUMS. On macOS it tells you when a release is out and points you back to this page.",
	copy: "Copy",
	copied: "Copied",
	platforms: {
		linux: {
			k: "Linux",
			v: "AppImage · x86_64",
			note: "Make it executable (`chmod +x`) and run it.",
		},
		macos: {
			k: "macOS",
			v: "Disk image · Apple silicon",
			note: "Signed and notarized. Open the disk image and drag Architect to Applications.",
		},
		node: {
			k: "Node",
			v: "x86_64 Linux · headless",
			note: "The app's own binary with no window, for a home server (`architect-desktop serve`).",
		},
	},
	/** An asset GitHub does not list (yet: a release's jobs upload them one by one; the Mac job is optional). */
	missing: "Not available for this release yet.",
	nix: {
		title: "On Nix",
		lede: "The flake's `architect-desktop` is the release's own AppImage in an FHS env, so installing it compiles nothing.",
		code: [
			{ comment: "the newest pinned release", line: NIX_RUN },
			{ comment: "this release", line: `${NIX_RUN}/v{version}` },
		],
		more: {
			label: "The desktop app on Nix, in the docs",
			href: `${DOCS_PATH}guide/shipping/#the-desktop-app-on-nix`,
		},
	},
	sha: "sha256",
	verify: {
		title: "Check a download",
		lede: "`SHA256SUMS` lists every file in the release with its sha256, and `SHA256SUMS.minisig` is its signature with the org's minisign key. In the folder you downloaded to:",
		code: [
			{
				comment: "the files you have, against the sums",
				line: "sha256sum --ignore-missing -c SHA256SUMS",
			},
			{
				comment: "the sums, against the org key",
				line: `minisign -Vm SHA256SUMS -P ${ORG_RELEASE_KEY}`,
			},
		],
	},
	sums: "SHA256SUMS",
	sig: "SHA256SUMS.minisig",
	bundle: "The web bundle ({file}), to serve this site yourself",
	all: "Every file in this release, on GitHub",
};

export const CHANGELOG = {
	title: "Changelog",
	description:
		"What changed in each Architect release: the release notes, built from the change files each pull request adds.",
	lede: "Each release's notes, built from the change files its pull requests added (`.changes/`), newest first.",
	onGitHub: "On GitHub",
	prerelease: "Pre-release",
	older: { label: "Older releases, on GitHub", href: `${SITE.repo}/releases` },
};

export const STORE = {
	title: "Extensions",
	description:
		"The extension store: what the Architect app can install from FastTrackStudio's registry.",
	lede: "What the Architect app can install from the FastTrackStudio store. Every archive is signed, and checked against its entry before it is listed.",
	fixture:
		"The store lists no extensions yet. Until it does, this page is built from the example registry (`examples/registry/index.toml`), so the entries below are an example, not something to install.",
	columns: { kind: "Kind", version: "Version", publisher: "Publisher" },
	kinds: {
		data: "Data: declarations only, settings, keys, views",
		luau: "Luau: actions as scripts",
		wasm: "WASI: a component in the app's sandbox",
		process: "Process: its own program",
	} as Record<string, string>,
	install: {
		title: "Install",
		app: "In Architect, open Extensions, find {name} in the {registry} store, and install it.",
		action:
			'Or run the action `EXT_INSTALL` with `{"id": "{id}"}` from ⌘K, a script, or an agent over MCP: it asks before granting what the extension needs.',
	},
	versions: "Versions",
	source: "Archive",
	platforms: "Platforms",
	back: "All extensions",
	list: {
		label: "List an extension",
		href: `${DOCS_PATH}architecture/extensions/`,
	},
};

export const NOT_FOUND = {
	title: "Nothing here",
	lede: "There is no page at this address.",
	links: [
		{ label: "Home", href: "/" },
		{ label: "Download", href: "/download" },
		{ label: "Docs", href: DOCS_PATH },
	],
};
