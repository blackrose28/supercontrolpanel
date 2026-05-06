
import { execSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

const projectDir = 'd:/Work/SuperControlPanel/cc-server/admin-next';
const configPath = path.join(projectDir, 'vite.config.ts');

try {
  console.log('Checking vite.config.ts types...');
  // We use npx tsc to check just the config file
  // We need to point to the node_modules of the project
  execSync(`npx tsc ${configPath} --noEmit --esModuleInterop --moduleResolution Bundler --target ESNext --skipLibCheck`, {
    cwd: projectDir,
    stdio: 'pipe'
  });
  console.log('Check passed!');
} catch (error: any) {
  console.error('Check failed with error:');
  console.error(error.stdout?.toString() || error.message);
  process.exit(1);
}
