import { builtinSettingsApi } from './builtinSettings';

/** Install updates (Settings → Updates): the built-in workflow's inputs. */
export const installUpdatesApi = { installUpdates: builtinSettingsApi('/api/install-updates') };
