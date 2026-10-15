import {exportNativePi, validateNativePiSession, type NativePiEntry, type NativePiSession} from '../pi/native.ts';
import {validateSnapshot, type Snapshot} from './snapshot.ts';

/** Reconstruct the entire original Pi tree by explicit source identities and
 * recorded source sequence, never by text, projected message position or count.
 * Refuses native-only Durable trees: those use selected-conversation conversion.
 */
export function exportPiSourceTree(snapshot: Snapshot) {
  validateSnapshot(snapshot);
  if (snapshot.selection !== 'full-tree') throw new Error('Full-tree export requires an explicitly full-tree snapshot');
  const originals = new Map<string, {entry: NativePiEntry; index: number}>();
  let header: NativePiSession['header'] | undefined;
  for (const conversation of snapshot.conversations) for (const entry of conversation.entries) {
    const data = entry.data as {sourcePiHeader?: NativePiSession['header']; hstryPiEntry?: NativePiEntry; sourcePiEntryIndex?: number} | undefined;
    if (data?.sourcePiHeader) {
      if (header && JSON.stringify(header) !== JSON.stringify(data.sourcePiHeader)) throw new Error('Conflicting source Pi headers');
      header = data.sourcePiHeader;
    }
    if (!data?.hstryPiEntry) {
      if (entry.kind !== 'hstry.pi.source') throw new Error('Native-only Durable entry requires selected-conversation export');
      continue;
    }
    if (!Number.isSafeInteger(data.sourcePiEntryIndex) || data.sourcePiEntryIndex! < 0) throw new Error('Missing immutable Pi source sequence');
    const record = {entry: data.hstryPiEntry, index: data.sourcePiEntryIndex!};
    const prior = originals.get(record.entry.id);
    if (prior && JSON.stringify(prior) !== JSON.stringify(record)) throw new Error('Conflicting Pi source entry identity');
    originals.set(record.entry.id, record);
  }
  if (!header) throw new Error('Missing original Pi header');
  const ordered = [...originals.values()].sort((a, b) => a.index - b.index);
  if (ordered.some((record, index) => record.index !== index)) throw new Error('Incomplete Pi source tree');
  const source = validateNativePiSession({header, entries: ordered.map(record => record.entry)});
  const result = exportNativePi({createdAt: Date.parse(header.timestamp), messages: [], externalId: header.id}, source);
  result.report.source.storeId = snapshot.storeId;
  const reportFile = result.files.find(file => file.path.endsWith('.fidelity.json'))!;
  reportFile.content = JSON.stringify(result.report);
  const sidecar = result.files.find(file => file.path.endsWith('.provenance.json'))!;
  sidecar.content = JSON.stringify({source, sourceSnapshot: snapshot, report: result.report});
  if (Buffer.byteLength(sidecar.content) > 16 * 1024 * 1024) throw new Error('Provenance sidecar exceeds 16 MiB');
  return result;
}
