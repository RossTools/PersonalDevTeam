import { builtinSettingsApi } from './builtinSettings';

/** "When an agent is stuck" (Settings → Agents): the built-in workflow's inputs. */
export const stuckAgentsApi = { stuckAgents: builtinSettingsApi('/api/stuck-agents') };
