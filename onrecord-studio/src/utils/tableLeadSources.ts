/**
 * The one reader of the "Tables read:" / "SQL:" tail that `analysis/service_leads.rs` appends to a
 * table-derived lead's stored explanation. A finding whose explanation has no such tail is not a
 * table lead and yields `null`; nothing is inferred from its wording.
 */

export interface TableLeadTable {
  /** The stored line, shown as is when its parts cannot be told apart. */
  line: string;
  parts: { name: string; source: string; retrievedOn: string; sha256: string } | null;
}

export interface TableLeadSources {
  tables: TableLeadTable[];
  sql: string;
}

const TABLES_HEADING = '\n\nTables read:\n';
const SQL_HEADING = '\n\nSQL:\n';
const RETRIEVED = ', retrieved ';
const DIGEST = ', sha256 ';
const SHA256_HEX_LENGTH = 64;

function tableParts(line: string): TableLeadTable['parts'] {
  const numbered = line.indexOf('. ');
  const digestAt = line.lastIndexOf(DIGEST);
  const retrievedAt = line.lastIndexOf(RETRIEVED, digestAt);
  if (numbered < 0 || digestAt < 0 || retrievedAt < 0) return null;
  const titled = line.slice(numbered + 2, retrievedAt);
  const sourceAt = titled.lastIndexOf("' (");
  const sha256 = line.slice(digestAt + DIGEST.length);
  if (!titled.startsWith("'") || !titled.endsWith(')') || sourceAt < 0 || sha256.length !== SHA256_HEX_LENGTH) return null;
  return {
    name: titled.slice(1, sourceAt),
    source: titled.slice(sourceAt + 3, -1),
    retrievedOn: line.slice(retrievedAt + RETRIEVED.length, digestAt),
    sha256,
  };
}

export function tableLeadSources(explanation: string): TableLeadSources | null {
  const tablesAt = explanation.lastIndexOf(TABLES_HEADING);
  if (tablesAt < 0) return null;
  const sqlAt = explanation.indexOf(SQL_HEADING, tablesAt);
  if (sqlAt < 0) return null;
  const lines = explanation.slice(tablesAt + TABLES_HEADING.length, sqlAt).split('\n').filter((line) => line.trim() !== '');
  const sql = explanation.slice(sqlAt + SQL_HEADING.length);
  if (lines.length === 0 || sql.trim() === '') return null;
  return { tables: lines.map((line) => ({ line, parts: tableParts(line) })), sql };
}

/** A table lead's own words: its explanation before the tables and SQL it was computed with. */
export function leadProse(explanation: string): string {
  const tablesAt = explanation.lastIndexOf(TABLES_HEADING);
  return tablesAt < 0 ? explanation : explanation.slice(0, tablesAt);
}
