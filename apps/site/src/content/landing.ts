/**
 * The landing page's words (#559). There are few of them on purpose: a
 * headline, the downloads, and a short heading over each real capture.
 * Every claim is true of the code at the commit it was written against;
 * what is not built yet is simply not said.
 */
import { DOCS_PATH, SITE } from "./site";

export type Media =
	| {
			kind: "video";
			src: string;
			poster: string;
			width: number;
			height: number;
			/** What the clip shows, for a reader who cannot see it. */
			alt: string;
	  }
	| {
			kind: "image";
			src: string;
			/** A narrower copy for phones, with its width. */
			small?: { src: string; width: number };
			width: number;
			height: number;
			alt: string;
	  };

export const HERO = {
	title: "The foundation you build on.",
	sub: "One local-first app for notes, tasks, files and projects. Every feature in it is an extension.",
	/** One button: the visitor's own platform, shown by its icon. */
	download: "Download",
	for: "for {platform}",
	linux: "Linux",
	macos: "macOS",
	more: { label: "More platforms", to: "/download" },
	demo: { label: "Demo", href: "/app/?demo" },
	/** The tabs over the screenshot. */
	pick: "Show",
	shots: [
		{
			id: "notes",
			label: "Notes",
			media: {
				kind: "image",
				src: "/media/hero-notes.webp",
				small: { src: "/media/hero-notes-840.webp", width: 840 },
				width: 1400,
				height: 875,
				alt: "A note called Garden open in Architect: a picture, a checklist with three of five items done, two callouts and a block of Rust code, beside the folder tree of the vault.",
			},
		},
		{
			id: "tasks",
			label: "Tasks",
			media: {
				kind: "image",
				src: "/media/hero-tasks.webp",
				small: { src: "/media/hero-tasks-840.webp", width: 840 },
				width: 1400,
				height: 875,
				alt: "The capture box with one typed line, and under it what was read from the line: a task, high priority, the project Website Redesign, the tag mixing, due November 8, repeating every week.",
			},
		},
		{
			id: "files",
			label: "Files",
			media: {
				kind: "image",
				src: "/media/hero-files.webp",
				width: 681,
				height: 757,
				alt: "A project's files in Architect: the REAPER project Crescendum with 27 files, and for the selected file, two verified copies at two sites, on the laptop and on the node.",
			},
		},
		{
			id: "projects",
			label: "Projects",
			media: {
				kind: "image",
				src: "/media/hero-projects.webp",
				small: { src: "/media/hero-projects-840.webp", width: 840 },
				width: 1400,
				height: 875,
				alt: "A project called Album: its songs with key and tempo, its notes, a sub-project and its tasks listed beside the project's own page.",
			},
		},
	] as ReadonlyArray<{ id: string; label: string; media: Media }>,
};

export type Feature = {
	id: string;
	title: string;
	media: readonly Media[];
	/** A short list under the heading: what it is built with, by name. */
	list?: {
		label: string;
		items: ReadonlyArray<{ name: string; note?: string }>;
	};
	/** A link under the heading, where there is somewhere to go. */
	link?: { label: string; href?: string; to?: string };
};

/** One heading and one capture (or two) each. */
export const FEATURES: readonly Feature[] = [
	{
		id: "extensions",
		title: "Build anything on it.",
		list: {
			label: "Write an extension in",
			items: [
				{ name: "Rust" },
				// Views are TSX today; actions in TypeScript are #536.
				{ name: "TypeScript", note: "views today" },
				{ name: "Lua" },
				{ name: "Anything that compiles to WASM" },
			],
		},
		link: {
			label: "How extensions work",
			href: `${DOCS_PATH}architecture/extensions/`,
		},
		media: [
			{
				kind: "video",
				src: "/media/extensions-manager.mp4",
				poster: "/media/extensions-manager.webp",
				width: 960,
				height: 573,
				alt: "The extensions manager: extensions, scripts, workers, themes, pages, layouts and keys in one list, with the Board extension turned off and on.",
			},
		],
	},
	{
		id: "vault",
		title: "A folder of markdown.",
		media: [
			{
				kind: "video",
				src: "/media/workspace-vault-tour.mp4",
				poster: "/media/vault-workspace.webp",
				width: 960,
				height: 573,
				alt: "The workspace: notes open as tabs, split side by side beside the graph and backlinks.",
			},
		],
	},
	{
		id: "calendar",
		title: "Plan the week.",
		media: [
			{
				kind: "image",
				src: "/media/calendar.webp",
				small: { src: "/media/calendar-840.webp", width: 840 },
				width: 1400,
				height: 875,
				alt: "A week in the calendar: each day planned in blocks from a day template, with a mix review, a team sync, a mastering call and a show on Saturday night.",
			},
		],
	},
	{
		id: "studio",
		title: "Studio, for music.",
		media: [
			{
				kind: "image",
				src: "/media/setlist.webp",
				small: { src: "/media/setlist-840.webp", width: 840 },
				width: 1400,
				height: 875,
				alt: "Architect with setlists in the navigator: Sunday Service holds three songs, and the song Slow River is open with its Keyflow chart as text and its key, tempo and status as properties.",
			},
			{
				kind: "image",
				src: "/media/studio-follow.webp",
				width: 730,
				height: 890,
				alt: "A shared song's page from Studio, Silent Night in A at 72 bpm: the recording is playing at 0:40 and bar 16 of the chart under it is marked.",
			},
		],
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
