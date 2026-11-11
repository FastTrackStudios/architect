/**
 * The landing page's words, ported verbatim from apps/web/landing/index.html.
 *
 * Inline markup in these strings is the small subset `<Rich>` reads
 * (src/components/ui/rich.tsx): `**bold**`, `` `code` `` and `[text](href)`.
 * The two drawings (the site plan and section A) keep their labels in their
 * components, beside the geometry they annotate.
 */
import { DOCS_PATH, RELEASES_LATEST, SITE } from "./site";

export type Sheet = { id: string; sn: string; title: string };

/** The sheet index: the bar's list, the title block's, and each sheet's strip. */
export const SHEETS: readonly Sheet[] = [
	{ id: "vault", sn: "A-101", title: "Vault" },
	{ id: "files", sn: "A-201", title: "Files" },
	{ id: "sync", sn: "A-301", title: "Sync" },
	{ id: "extensions", sn: "A-401", title: "Extensions" },
	{ id: "studio", sn: "M-101", title: "Studio" },
	{ id: "self-host", sn: "S-101", title: "Self-host" },
	{ id: "framework", sn: "D-101", title: "Framework" },
];

export const HERO_SHEET = "A-000 Site plan";

export const BAR = {
	sheetsLabel: "Sheets",
	links: [
		{ label: "Guides", href: "/guides/" },
		{ label: "Docs", href: DOCS_PATH },
	],
	plot: "Plot",
	model: "Model",
	plotHint: ": switch between the light and dark drawing",
	skip: "Skip to the content",
};

export type Action = {
	k: string;
	v?: string;
	href: string;
	primary?: boolean;
	arrow: "down" | "right";
};

/** "Get Architect": the hero's and the issue sheet's buttons. */
export const GET_ACTIONS: readonly Action[] = [
	{
		k: "Download",
		v: "Linux AppImage · macOS",
		href: RELEASES_LATEST,
		primary: true,
		arrow: "down",
	},
	{
		k: "Try the demo",
		v: "The demo vault, in your browser",
		href: "/app/?demo",
		arrow: "right",
	},
	{
		k: "Open the web app",
		v: "Sign in to your org",
		href: "/app/",
		arrow: "right",
	},
];

export type Room = "node" | "desk" | "laptop" | "phone" | "web";
export type Keynote = { title: string; body: string };

export const HERO = {
	title: "Architect",
	lede: "Your notes, projects and files on every device. Local-first: work stays on the device, and your devices sync with each other directly.",
	planCaption:
		"from the app's demo data: one musician's devices around the always-on node. Dashed means offline.",
	planCaptionLabel: "Example plot",
	planMore: ["← Desk · Phone", "match line", "Laptop · Web →"],
	keynotes: [
		{
			room: "node",
			title: "Node.",
			body: "The Architect app run headless on a home server. One more peer, the one that is always online, holding every project in full.",
		},
		{
			room: "desk",
			title: "Desk.",
			body: "Pins the projects it works on. Each copy is checked by a possession challenge, and the count is shown.",
		},
		{
			room: "laptop",
			title: "Laptop.",
			body: "Offline, and still working. Its edits merge when it is back; nothing waits on a server.",
		},
		{
			room: "phone",
			title: "Phone.",
			body: "The same vault and the same workspace, on the mobile app.",
		},
		{
			room: "web",
			title: "Web.",
			body: "The app in a browser at `/app/`, signed in to your org.",
		},
	] as ReadonlyArray<Keynote & { room: Room }>,
	data: [
		{ dt: "Runs on", dd: "Linux · macOS · mobile · web" },
		{ dt: "Built with", dd: "architect, in Rust" },
		{ dt: "Licence", dd: SITE.license },
		{ dt: "Status", dd: "Pre-1.0" },
	],
	indexLabel: "Sheet index",
	sheet: { k: "Sheet", sn: "A-000", title: "Site plan" },
};

export type Media =
	| {
			kind: "video";
			src: string;
			poster: string;
			width: number;
			height: number;
			label: string;
	  }
	| {
			kind: "image";
			src: string;
			width: number;
			height: number;
			alt: string;
	  };

export type Detail = { media: Media; n: number; caption: string };

export const VAULT = {
	title: "A vault is a folder of markdown.",
	lede: "Architect opens a folder of notes, an Obsidian vault as it is, in a workspace of tabs, splits and docks. Everything below is kept as notes in that folder, so it syncs like the rest.",
	tour: {
		n: 1,
		media: {
			kind: "video",
			src: "/media/workspace-vault-tour.mp4",
			poster: "/media/vault-workspace.webp",
			width: 960,
			height: 573,
			label:
				"The workspace: notes open as tabs, split side by side beside the graph and backlinks.",
		},
		caption:
			"The workspace. Notes open as tabs; drag one to a pane's edge to split it. What you arrange comes back the next time you open the vault.",
	} as Detail,
	keynotesLabel: "What a vault holds",
	keynotes: [
		{
			title: "Notes and Bases.",
			body: "Markdown notes with properties, links and embeds. A `.base` file is a query over them, drawn as a table, cards or a list.",
		},
		{
			title: "Navigator.",
			body: "A folder tree beside the notes in the selected folder: Notebook Navigator, the Obsidian plugin, ported spec for spec.",
		},
		{
			title: "Calendar.",
			body: "Your events, and with Tasks and the Time Tracker, your tasks on their days and your tracked time. A day opens its daily note.",
		},
		{
			title: "Tasks and projects.",
			body: "A task is a note with `type: task`, in a list, Today, My tasks or a board. Projects keep goals and milestones as notes.",
		},
		{
			title: "Time tracking.",
			body: "Time your work on tasks and projects, and add it up.",
		},
		{
			title: "Capture.",
			body: "One line from anywhere: `[[Project]]`, `#tag`, `!high`, `next fri`, `every week` are read as you type.",
		},
		{
			title: "Threads and notifications.",
			body: "Conversations on any note, project or task, with mentions and resolve; notifications that reach every device.",
		},
	] as readonly Keynote[],
	inbox: {
		n: 2,
		media: {
			kind: "image",
			src: "/media/capture-inbox.webp",
			width: 1280,
			height: 750,
			alt: "The capture inbox: five captured lines to sort, projects in the sidebar, and the selected item's details with Open, Done and Move to.",
		},
		caption: "The inbox, where captured lines wait to be sorted into projects.",
	} as Detail,
	phone: {
		n: 3,
		media: {
			kind: "image",
			src: "/media/phone-tabs.webp",
			width: 402,
			height: 874,
			alt: "The phone app's tab switcher: four open tabs as cards, Welcome, The app shell, Arrangement and Graph.",
		},
		caption: "The same workspace on a phone.",
	} as Detail,
};

export const FILES = {
	title: "Every file held at home, pinned where you work.",
	lede: "An org's projects and asset libraries live on the node. Each machine pins the ones it needs and fetches the rest on demand. A registry knows every file on every device, and how many verified copies of it exist.",
	cutCaption:
		"Section A, drawn from the file page below. The count is of copies a holder proved it has, not copies it says it has.",
	keynotesLabel: "How files are kept",
	keynotes: [
		{
			title: "A real folder.",
			body: "The drive mounts your roots at `~/Architect/<org>/Projects/…`, so REAPER, a file manager or `rsync` reads and writes them like any other folder.",
		},
		{
			title: "Pinned, or on demand.",
			body: "Bytes are kept by SHA-256 and fetched from a peer or a server when opened, resuming after a dropped link.",
		},
		{
			title: "Versions, not a trash folder.",
			body: "Every save is a version, and so is a delete. Uploads resume and skip bytes the org already holds.",
		},
		{
			title: "Copies you can count.",
			body: "Holders sign what they hold, scrubs and possession challenges check it, and a policy says how many verified copies each file needs.",
		},
		{
			title: "S3 when you want it.",
			body: "Nodes can keep bytes in any S3-compatible store: Garage, MinIO, R2, AWS S3, Ceph.",
		},
		{
			title: "Found offline.",
			body: "Search finds every file on every device from the registry, even with every peer offline.",
		},
	] as readonly Keynote[],
	details: [
		{
			n: 1,
			media: {
				kind: "image",
				src: "/media/file-copies.webp",
				width: 366,
				height: 616,
				alt: "A project page, Crescendum Album: five audio files, and for the selected file its copies: two verified copies at two sites, desk and node, with the laptop offline and unverified.",
			},
			caption: "A file's copies, on its project page.",
		},
		{
			n: 2,
			media: {
				kind: "image",
				src: "/media/storage-health.webp",
				width: 366,
				height: 580,
				alt: "Storage health for an org: 7.7 MB of files, 6.5 MB under policy, 352.8 KB on one holder, nothing without a verified copy, and the holders with what each has verified.",
			},
			caption: "Storage health: what is under policy, and on whom.",
		},
	] as readonly Detail[],
};

export const SYNC = {
	title: "Devices sync with each other. The server is one of them.",
	lede: "There is no separate Architect server. Every device is a peer, and the always-on node is just the one that never sleeps. Stop it, and every other device keeps working.",
	tableLabel: "How Architect syncs",
	columns: ["Mark", "What", "How it works"],
	rows: [
		{
			mark: "P-1",
			what: "Peer to peer",
			how: "Devices exchange documents directly over iroh, finding each other on the LAN or through a relay. Each document checks who may read it.",
		},
		{
			mark: "P-2",
			what: "The always-on node",
			how: "Keeps a live bridge per note with each peer it reaches, so an edit gets to the others within a second.",
		},
		{
			mark: "P-3",
			what: "Live collaboration",
			how: "Presence shows who is in a document and what they point at, select and type; text merges as you both write.",
		},
		{
			mark: "P-4",
			what: "Share links",
			how: "Share a vault, a folder, a tag, a few notes or what a `.base` selects, to the public, anyone with the link, your org or named people. Read-only or with comments, until it expires or you revoke it.",
		},
		{
			mark: "P-5",
			what: "Published sites",
			how: `A vault becomes a static site of finished HTML, rebuilt when the vault changes. [These docs](${DOCS_PATH}) are the first site built with it.`,
		},
		{
			mark: "P-6",
			what: "Your devices, admitted",
			how: "Your own devices, members an org enrolled with a role, and devices paired by a six-digit code or a QR code. Revoke one and its next call is refused.",
		},
	],
	strip: "Schedule",
};

export const EXTENSIONS = {
	title: "Every feature is an extension.",
	lede: "Calendar, Tasks, Projects and the Time Tracker are extensions, built the way yours would be. Turn one off and the app goes on without it. Every command is one action on the Action List, run the same way by keys, menus, ⌘K, scripts, extensions and agents over MCP.",
	manager: {
		n: 1,
		media: {
			kind: "video",
			src: "/media/extensions-manager.mp4",
			poster: "/media/extensions-manager.webp",
			width: 960,
			height: 573,
			label:
				"The extensions manager: extensions, Luau scripts, workers, themes, pages, layouts and keys in one list, with the Board extension turned off and on.",
		},
		caption:
			"Extensions, scripts, workers, themes, layouts and pages, in one manager.",
	} as Detail,
	palette: {
		n: 2,
		media: {
			kind: "image",
			src: "/media/palette-actions.webp",
			width: 418,
			height: 404,
			alt: "The ⌘K palette with 'count in' typed: the Metronome extension's count-in action first, then actions from the Board, Settings and Extensions.",
		},
		caption: "⌘K runs any action, an extension's included.",
	} as Detail,
	keynotesLabel: "Kinds of extension",
	keynotes: [
		{ title: "Data.", body: "Declarations only: settings, keys, views." },
		{ title: "Luau.", body: "Actions as scripts, run as one undo step." },
		{
			title: "Process.",
			body: "Its own program, with views drawn in the app's window.",
		},
		{
			title: "Hot reload.",
			body: "Loaded from a registry, reloaded as you edit it.",
		},
	] as readonly Keynote[],
};

export const STUDIO = {
	title: "Built for Studio.",
	lede: "FastTrackStudio's music apps, Studio, Session, Signal and Keyflow, are built on architect. The same vault holds the setlists, charts and session notes; the same drive holds the audio, where REAPER can open it.",
	appsLabel: "FastTrackStudio apps on architect",
	apps: ["Studio", "Session", "Signal", "Keyflow"],
	chart: {
		n: 1,
		media: {
			kind: "video",
			src: "/media/embeds-in-a-note.mp4",
			poster: "/media/keyflow-note.webp",
			width: 960,
			height: 570,
			label:
				"A Keyflow chart in a note, drawn by the Board extension; turned off, the chart goes back to a plain code fence.",
		},
		caption:
			"A Keyflow chart in a note, drawn by an extension. Turn it off and the fence is plain text again.",
	} as Detail,
	keynotesLabel: "What Studio's apps share",
	keynotes: [
		{
			title: "Charts in notes.",
			body: "A Keyflow fence in a note is drawn as a chart by the extension that claims it.",
		},
		{
			title: "Audio on the drive.",
			body: "Session files sit in a real folder, so REAPER opens them like any other.",
		},
		{
			title: "Presence.",
			body: "Pointers name a place, a track and a time, not pixels, so each app draws them in its own view.",
		},
	] as readonly Keynote[],
};

export const SELF_HOST = {
	title: "Your server is the app, run headless.",
	lede: "The node is the same binary with no window. Run it on a home server and it holds your org's projects, serves your sites and share links, and gives agents MCP. It is released as a container image built with Nix.",
	notesTitle: "General notes",
	code: [
		{ comment: "# the node, from the app's own binary" },
		{ line: "architect-desktop serve --config architect.toml" },
		{ line: "" },
		{ comment: "# or the image, published with each release" },
		{ line: "ghcr.io/fasttrackstudios/architect:latest" },
	],
	spec: [
		{ dt: "8080/tcp", dd: "HTTP: sites, share links, the web lanes, MCP" },
		{ dt: "7777/udp", dd: "iroh, for peers that reach it directly" },
		{ dt: "/data", dd: "its key, documents and built sites" },
		{ dt: "GET /health", dd: "answers `ok`" },
	],
	guide: { label: "Read the deploy guide", href: `${DOCS_PATH}guide/deploy/` },
	strip: "General notes",
};

export const FRAMEWORK = {
	title: "The framework underneath.",
	lede: "architect is the Rust framework the app is built from. Declare each service and document once, and it gives you every wire, offline and peer-to-peer sync, and one Action List that keys, menus, scripts, extensions and MCP all run.",
	codeCaption:
		"From this one trait: the vox service, `POST /greeter/greet`, `/greeter/greeted` as Server-Sent Events, `GreeterClient` and `GreeterHttpClient`, and the action `GREETER_GREET` for keys, menus, scripts and MCP.",
	views: {
		n: 1,
		media: {
			kind: "image",
			src: "/media/embedded-views.webp",
			width: 960,
			height: 573,
			alt: "Four embedded views in the workspace: a Dioxus component, a Frame surface with a drive knob and meter, a Luau script's immediate-mode window, and an agent's A2UI checklist.",
		},
		caption:
			"Views from Dioxus, Frame, a Luau script and an agent's A2UI, side by side in one workspace.",
	} as Detail,
	keynotesLabel: "What the framework gives you",
	keynotes: [
		{
			title: "Contracts.",
			body: "Services and entities declared once: vox in process, over WebSocket and iroh, and HTTP with JSON and SSE.",
		},
		{
			title: "Documents.",
			body: "Loro CRDTs with kinds, migrations and named undo, in a crash-safe file per document.",
		},
		{
			title: "Sync.",
			body: "Org sync, peer to peer over iroh, presence and collaborative text.",
		},
		{
			title: "The shell.",
			body: "Tab groups in splits, docks, pop-out windows and native menus, with key bindings, chords and which-key.",
		},
	] as readonly Keynote[],
	docs: { label: "Read the docs", href: DOCS_PATH },
	source: { label: "Source on GitHub", href: SITE.repo },
	strip: "Pre-1.0: any release may break the API",
};

export const ISSUE = {
	title: "Issued for use.",
	read: [
		{ label: "Guides, for using the app", href: "/guides/" },
		{ label: "Docs, for building on architect", href: DOCS_PATH },
	],
};

export const COLOPHON = {
	caption: "Revisions",
	columns: ["Rev", "Description", "By"],
	rows: [
		{
			rev: "Latest",
			label: "Release notes on GitHub",
			href: RELEASES_LATEST,
			by: "FastTrackStudio",
		},
	],
	byline: { pre: "Architect by", org: "FastTrackStudio", href: SITE.org },
	tail: ` · ${SITE.license} · Set in Archivo and osifont`,
};

/** The strip at the foot of most sheets. */
export const NOT_TO_SCALE = "Scale: not to scale";
