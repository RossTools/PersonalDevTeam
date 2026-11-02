// A document provider's own file picker (PDT-378): a module the provider serves (`ProviderPicker.module`), loaded
// once and run for an account. It resolves to the references of the files picked, none when cancelled.
import { api } from '../api/client';

export interface PickerModule {
  pick(opts: { account: string; request: (path: string) => Promise<unknown> }): Promise<string[]>;
}

/** Loads a picker's module (tests stand one in). */
export const pickerModules = {
  load: (url: string): Promise<PickerModule> => import(/* @vite-ignore */ url) as Promise<PickerModule>,
};

/** Opens the picker at `module` as `account`: the references picked. */
export async function pickFiles(module: string, account: string): Promise<string[]> {
  const m = await pickerModules.load(module);
  return m.pick({ account, request: api.pickerRequest });
}
