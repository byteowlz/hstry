// Find-as-you-type matching; same rules as the TUI (crates/hstry-tui/src/fuzzy.rs):
// every term must match in order; contiguous and word-start runs rank higher;
// a title hit outranks a metadata hit; wide scatter is not a match.

const BOUNDARY = new Set([" ", "/", "-", "_", ".", ":", "(", "[", '"', "'"]);

function isBoundary(hay: string[], pos: number): boolean {
	return pos === 0 || BOUNDARY.has(hay[pos - 1] ?? "");
}

function findContiguous(hay: string[], needle: string[]): number {
	outer: for (let i = 0; i + needle.length <= hay.length; i++) {
		for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
		return i;
	}
	return -1;
}

/** Score `needle` as an ordered subsequence of `hay` (both lowercased chars). */
export function subsequence(hay: string[], needle: string[]): { score: number; positions: number[] } | null {
	if (needle.length === 0) return { score: 0, positions: [] };
	const start = findContiguous(hay, needle);
	if (start >= 0) {
		const positions = needle.map((_, i) => start + i);
		const score = 10 * needle.length + (isBoundary(hay, start) ? 15 : 0) + (start === 0 ? 10 : 0);
		return { score, positions };
	}
	const positions: number[] = [];
	let score = 0;
	let from = 0;
	let prev = -2;
	for (const nc of needle) {
		let pos = -1;
		for (let i = from; i < hay.length; i++) {
			if (hay[i] === nc) {
				pos = i;
				break;
			}
		}
		if (pos < 0) return null;
		score += 1;
		if (pos === prev + 1) score += 4;
		if (isBoundary(hay, pos)) score += 5;
		positions.push(pos);
		prev = pos;
		from = pos + 1;
	}
	const span = positions[positions.length - 1]! - positions[0]!;
	if (span > needle.length * 3 + 2) return null;
	score -= Math.min(Math.floor(span / 4), 10);
	return { score, positions };
}

export interface MatchResult {
	score: number;
	/** Character indices into the title to highlight. */
	titlePositions: number[];
}

const chars = (s: string) => Array.from(s.toLowerCase());

/** All query terms must match the title or the secondary (metadata) text. */
export function matchQuery(query: string, title: string, secondary: string): MatchResult | null {
	const terms = query.split(/\s+/).filter(Boolean).map(chars);
	if (terms.length === 0) return { score: 0, titlePositions: [] };
	const t = chars(title);
	const s = chars(secondary);
	let score = 0;
	const highlight = new Set<number>();
	for (const term of terms) {
		const inTitle = subsequence(t, term);
		let inSecondary = subsequence(s, term);
		// Metadata is identifiers: abbreviations may skip letters inside one
		// word, never jump across words.
		if (inSecondary) {
			const first = inSecondary.positions[0]!;
			const last = inSecondary.positions[inSecondary.positions.length - 1]!;
			if (s.slice(first, last + 1).some((c) => /\s/.test(c))) inSecondary = null;
		}
		if (inTitle) {
			const titleScore = inTitle.score + 20;
			if (titleScore >= (inSecondary?.score ?? Number.NEGATIVE_INFINITY)) {
				score += titleScore;
				for (const p of inTitle.positions) highlight.add(p);
			} else {
				score += inSecondary!.score;
			}
		} else if (inSecondary) {
			score += inSecondary.score;
		} else {
			return null;
		}
	}
	return { score, titlePositions: [...highlight].sort((a, b) => a - b) };
}
