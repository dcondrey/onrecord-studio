import Table, { Td, Th } from '../../ui/Table';
import { formatValue, type ValueKind } from '../core/format';
import { ABSENCE_LABEL, isAbsence, type Absence } from '../core/nulls';

export type TwinCell = string | number | null | Absence;

export interface TwinColumn {
  key: string;
  label: string;
  kind?: ValueKind;
  align?: 'start' | 'end';
}

export interface TwinTable {
  caption: string;
  columns: TwinColumn[];
  /** Exactly the plotted rows, plus every excluded and absent row with its reason. */
  rows: Record<string, TwinCell>[];
}

/** The text a twin cell prints. An absence prints its state and reason, never a blank or a 0. */
export function twinCellText(cell: TwinCell | undefined, kind?: ValueKind): string {
  if (cell === null || cell === undefined) return '—';
  if (isAbsence(cell)) return `${ABSENCE_LABEL[cell.state]}: ${cell.reason}`;
  if (typeof cell === 'number') return kind ? formatValue(cell, { kind }) : cell.toLocaleString('en-US');
  return cell;
}

/** The chart's data as a real, copyable table. */
export function TableTwin({ table }: { table: TwinTable }) {
  return (
    <Table label={table.caption}>
      <caption className="sr-only">{table.caption}</caption>
      <thead>
        <tr>
          {table.columns.map((column) => (
            <Th key={column.key} numeric={(column.align ?? (column.kind ? 'end' : 'start')) === 'end'}>{column.label}</Th>
          ))}
        </tr>
      </thead>
      <tbody>
        {table.rows.map((row, index) => (
          <tr key={index}>
            {table.columns.map((column) => {
              const cell = row[column.key];
              const absent = isAbsence(cell);
              return (
                <Td
                  key={column.key}
                  numeric={!absent && (column.align ?? (column.kind ? 'end' : 'start')) === 'end'}
                  className={absent ? 'text-caption text-muted' : 'viz-num'}
                >
                  {twinCellText(cell, column.kind)}
                </Td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
