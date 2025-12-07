#!/usr/bin/env node

const domino = require('../index.js');
const path = require('path');

function parseArgs() {
  const args = process.argv.slice(2);
  const options = {
    command: null,
    base: 'origin/main',
    cwd: process.cwd(),
    json: false,
    all: false,
    tsConfig: null,
    debug: false,
    ci: false,
    profile: false,
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    const nextArg = args[i + 1];

    if (arg === 'affected') {
      options.command = 'affected';
    } else if (arg === '--base' && nextArg) {
      options.base = nextArg;
      i++;
    } else if (arg === '--cwd' && nextArg) {
      options.cwd = path.resolve(nextArg);
      i++;
    } else if (arg === '--json') {
      options.json = true;
    } else if (arg === '--all') {
      options.all = true;
    } else if (arg === '--ts-config' && nextArg) {
      options.tsConfig = path.resolve(nextArg);
      i++;
    } else if (arg === '--debug' || arg === '-d') {
      options.debug = true;
    } else if (arg === '--ci') {
      options.ci = true;
    } else if (arg === '--profile') {
      options.profile = true;
    } else if (arg === '--help' || arg === '-h') {
      console.log(`
Usage: domino affected [options]

Commands:
  affected    Find affected projects

Options:
  --base <BRANCH>        Base branch to compare against (default: origin/main)
  --cwd <PATH>           Current working directory
  --json                 Output results as JSON
  --all                  Show all projects regardless of changes
  --ts-config <PATH>     Path to root tsconfig
  --debug, -d            Enable debug logging
  --ci                   CI mode: suppress all logs, only output results
  --profile              Enable performance profiling
  --help, -h             Show this help message
`);
      process.exit(0);
    }
  }

  return options;
}

function detectDefaultBranch(cwd) {
  try {
    const { execSync } = require('child_process');
    const result = execSync('git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null || git branch -r | grep "origin/HEAD" | head -1', {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    const match = result.trim().match(/origin\/(\w+)/);
    if (match) {
      return `origin/${match[1]}`;
    }
  } catch (err) {
    // Fallback to origin/main
  }
  return 'origin/main';
}

async function runAffected(options) {
  const cwd = options.cwd || process.cwd();

  // Auto-detect default branch if using the default value
  let base = options.base;
  if (base === 'origin/main') {
    base = detectDefaultBranch(cwd);
  }

  // Enable profiling via --profile flag or DOMINO_PROFILE env var
  const enableProfiling = options.profile || process.env.DOMINO_PROFILE === '1';
  if (enableProfiling && !options.ci) {
    console.error('📊 Performance profiling enabled');
  }

  // Discover projects
  const projects = domino.discoverProjects(cwd);

  if (projects.length === 0) {
    if (options.json) {
      console.log('[]');
    } else if (!options.ci) {
      console.error('No projects found in workspace');
    }
    return;
  }

  if (options.all) {
    const allProjects = projects.map((p) => p.name);

    if (options.json) {
      console.log(JSON.stringify(allProjects));
    } else if (!options.ci) {
      console.log('All projects:');
      for (const project of allProjects) {
        console.log(`  • ${project}`);
      }
      console.log(`\nTotal: ${allProjects.length} projects`);
    } else {
      console.log(JSON.stringify(allProjects));
    }
    return;
  }

  // Run true-affected analysis
  const result = domino.findAffected({
    cwd,
    base,
    rootTsConfig: options.tsConfig,
    projects,
    include: [],
    ignoredPaths: ['node_modules', 'dist', 'build', '.git'],
    enableProfiling,
  });

  if (options.json) {
    console.log(JSON.stringify(result.affectedProjects));
  } else if (result.affectedProjects.length === 0) {
    if (!options.ci) {
      console.log('No affected projects');
    }
  } else {
    if (!options.ci) {
      console.log('Affected projects:');
      for (const project of result.affectedProjects) {
        console.log(`  • ${project}`);
      }
      console.log(
        `\nTotal: ${result.affectedProjects.length} affected project${result.affectedProjects.length === 1 ? '' : 's'}`,
      );
    } else {
      console.log(JSON.stringify(result.affectedProjects));
    }
  }
}

async function main() {
  const options = parseArgs();

  if (!options.command) {
    console.error('Error: No command specified');
    console.error('Run "domino --help" for usage information');
    process.exit(1);
  }

  try {
    switch (options.command) {
      case 'affected':
        await runAffected(options);
        break;
      default:
        console.error(`Error: Unknown command "${options.command}"`);
        console.error('Run "domino --help" for usage information');
        process.exit(1);
    }
  } catch (error) {
    console.error(`Error: ${error.message}`);
    if (options.debug) {
      console.error(error.stack);
    }
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}
