import { builtinSettingsApi } from './builtinSettings';

/** The PM daily checkpoint (Settings → This host → PM): the built-in workflow's inputs. */
export const pmCheckpointApi = { pmCheckpoint: builtinSettingsApi('/api/pm-checkpoint') };
