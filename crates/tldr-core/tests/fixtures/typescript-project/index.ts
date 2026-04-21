/**
 * Main entry point for the TypeScript project.
 */

import { processItems, ItemProcessor } from './processor';

export function main(): number {
    const items = [1, 2, 3, 4, 5];
    const result = processItems(items);
    console.log(`Result: ${result}`);
    return result;
}

export async function asyncMain(): Promise<number> {
    const processor = new ItemProcessor();
    await processor.initialize();
    return processor.run([1, 2, 3]);
}

// Unused function - dead code
export function unusedExport(): void {
    console.log('never called');
}
