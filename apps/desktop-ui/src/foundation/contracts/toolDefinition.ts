/**
 * OpenAI function-calling 形态的工具 schema（provider 与 ToolSurface 共用）。
 */

export interface ToolDefinition {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  };
}
