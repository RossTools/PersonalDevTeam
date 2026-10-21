import { describe, expect, it } from 'vitest';
import { branchOptions } from './branchOptions';

const b = (name: string, local = true) => ({ name, local });

describe('branchOptions', () => {
  it('puts main first, then master and the given branch, the rest in order', () => {
    const got = branchOptions([b('a'), b('dev'), b('master'), b('z', false), b('main')], 'dev');
    expect(got.map((o) => o.value)).toEqual(['main', 'master', 'dev', 'a', 'z']);
    expect(got.find((o) => o.value === 'z')?.label).toBe('z (origin)');
  });

  it('adds the given branch when the list lacks it', () => {
    expect(branchOptions([b('a'), b('main')], 'gone').map((o) => o.value)).toEqual(['gone', 'main', 'a']);
    expect(branchOptions([b('a')]).map((o) => o.value)).toEqual(['a']);
  });
});
