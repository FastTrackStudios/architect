import changelogJson from "#/data/changelog.json";
import type { ReleaseNotes } from "./data";

/** The last releases' notes (src/data/changelog.json), newest first. */
export const CHANGELOG_RELEASES = changelogJson.releases as ReleaseNotes[];
