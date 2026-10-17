export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  commands: Array<{
    id: string;
    title: string;
    shortcut?: string;
  }>;
}
