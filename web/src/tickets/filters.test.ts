import { describe, expect, it } from 'vitest';
import { DEFAULT_FILTERS, filterCount, isDefaultFilter, normaliseFilters, stagesText, toQuery } from './model';

describe('backlog filters', () => {
  it('starts with Draft and Ready, explicitly', () => {
    expect(DEFAULT_FILTERS.stages).toEqual(['draft', 'ready']);
    expect(toQuery(null, DEFAULT_FILTERS)).toEqual({ limit: 100, stage: 'draft,ready' });
    expect(isDefaultFilter({ ...DEFAULT_FILTERS, stages: ['ready', 'draft'] })).toBe(true);
    expect(filterCount({ ...DEFAULT_FILTERS, stages: ['ready'] })).toBe(1);
    expect(isDefaultFilter({ ...DEFAULT_FILTERS, group: 'stage', sort: 'updated' })).toBe(true);
  });

  it('asks for stages in stage order, and for none when every stage is picked or none is', () => {
    expect(toQuery(null, { ...DEFAULT_FILTERS, stages: ['in_progress', 'draft'] }).stage).toBe('draft,in_progress');
    expect(toQuery(null, { ...DEFAULT_FILTERS, stages: [] })).not.toHaveProperty('stage');
    expect(toQuery(null, { ...DEFAULT_FILTERS, stages: ['draft', 'ready', 'todo', 'in_progress', 'done', 'wont_do'] })).not.toHaveProperty('stage');
  });

  it('words the stage filter', () => {
    expect(stagesText(['draft', 'ready'])).toBe('Draft, Ready');
    expect(stagesText(['todo', 'draft', 'in_progress', 'ready'])).toBe('All open');
    expect(stagesText([])).toBe('All');
    expect(stagesText(['draft', 'ready', 'done'])).toBe('3 stages');
  });

  it('reads filters stored before the default was explicit', () => {
    expect(normaliseFilters({ stages: [], showClosed: false } as never).stages).toEqual(['draft', 'ready']);
    expect(normaliseFilters({ stages: [], showClosed: true } as never).stages).toEqual([]);
    expect(normaliseFilters({ stages: ['done'], showClosed: true } as never)).not.toHaveProperty('showClosed');
    expect(normaliseFilters({ stages: [] }).stages).toEqual([]);
    expect(normaliseFilters({}).stages).toEqual(['draft', 'ready']);
  });
});
