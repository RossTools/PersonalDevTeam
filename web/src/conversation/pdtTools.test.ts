import { describe, expect, it } from 'vitest';
import { editsText, editText, lookedUp } from './pdtTools';

const out = (v: unknown) => JSON.stringify(v);

describe('editText', () => {
  it('says what each document edit does', () => {
    expect(editText({ op: 'replace_text', markdown: '# New' })).toBe('rewrote the whole document');
    expect(editText({ op: 'insert_after_heading', heading: 'Scope' })).toBe('added text under “Scope”');
    expect(editText({ op: 'replace_string', find: 'cart' })).toBe('replaced “cart”');
    expect(editText({ op: 'replace_string', find: 'cart', all: true })).toBe('replaced “cart” everywhere');
    expect(editText({ op: 'table_set_cell', heading: 'Plan' })).toBe('changed a table cell under “Plan”');
    expect(editText({ op: 'table_set_cell' })).toBe('changed a table cell');
  });

  it('says what each sheet edit does', () => {
    expect(editText({ op: 'rename_sheet', sheet: 'Q3', name: 'Q4' })).toBe('renamed “Q3” to “Q4”');
    expect(editText({ op: 'delete_sheet', sheet: 'Old' })).toBe('deleted the sheet “Old”');
    expect(editText({ op: 'format_range', range: 'A1:B2' })).toBe('formatted A1:B2');
    expect(editText({ op: 'set_borders', range: 'C1:C9' })).toBe('set borders on C1:C9');
    expect(editText({ op: 'conditional_format', range: 'D2:D20' })).toBe('added a conditional format on D2:D20');
    expect(editText({ op: 'update_chart', id: 'c1' })).toBe('changed a chart');
    expect(editText({ op: 'delete_chart', id: 'c1' })).toBe('deleted a chart');
    expect(editText({ op: 'column_width', columns: 'B:D' })).toBe('resized column B:D');
    expect(editText({ op: 'clear_filter', sheet: 'Q3' })).toBe('cleared a filter');
  });

  it('fills in what a call leaves out', () => {
    for (const op of ['format_range', 'set_borders', 'conditional_format']) expect(editText({ op })).toMatch(/a range$/);
    expect(editText({ op: 'rename_sheet' })).toBe('renamed “?” to “?”');
    expect(editText({ op: 'delete_sheet', sheet: '  ' })).toBe('deleted the sheet “?”');
    expect(editText({ op: 'column_width' })).toBe('resized column ?');
    // An op this version doesn't know reads as its words; no op at all, or no object, says it changed.
    expect(editText({ op: 'merge_cells' })).toBe('merge cells');
    expect(editText({})).toBe('changed it');
    expect(editText(null)).toBe('changed it');
  });

  it('lists up to three edits, then counts the rest', () => {
    expect(editsText([{ op: 'format_range', range: 'A1' }, { op: 'set_borders', range: 'A1' }, { op: 'clear_filter' }]))
      .toBe('Formatted A1, set borders on A1 and cleared a filter');
    expect(editsText('not a list')).toBeNull();
  });
});

describe('lookedUp', () => {
  it("reads search_tickets' matches and get_overview's tickets in progress, from Codex's result too", () => {
    expect(lookedUp([{ tool: 'mcp:pdt/search_tickets', input: { query: 'x' }, output: out({ structuredContent: { tickets: [{ key: 'PDT-1' }, { title: 'no key' }] } }) }]))
      .toEqual(['PDT-1']);
    expect(lookedUp([{ tool: 'mcp__pdt__get_overview', input: {}, output: out({ content: [{ type: 'text', text: out({ inProgress: [{ key: 'PDT-9' }] }) }] }) }]))
      .toEqual(['PDT-9']);
  });

  it('names nothing for a lookup that has not answered, failed, or read no tickets', () => {
    expect(lookedUp([{ tool: 'mcp__pdt__search_tickets', input: { query: 'x' } }])).toBeNull();
    expect(lookedUp([{ tool: 'mcp__pdt__get_overview', input: {}, output: 'Error: no such project' }])).toBeNull();
    expect(lookedUp([{ tool: 'mcp__pdt__get_overview', input: {}, output: out({ inProgress: 'none' }) }])).toBeNull();
    // Another server's tool of the same name isn't PDT's.
    expect(lookedUp([{ tool: 'mcp__jira__search_tickets', input: {}, output: out({ tickets: [{ key: 'J-1' }] }) }])).toBeNull();
  });
});
