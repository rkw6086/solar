import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const script = fileURLToPath(new URL('./ci/cache-paths.mjs', import.meta.url));
const sha = 'a'.repeat(40);
const nativeRunner = {linux:'Linux',darwin:'macOS',win32:'Windows'}[process.platform];

function fixture(callback) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'solar-cache-test-'));
  const commandLog = path.join(root, 'commands.log');
  const fakeSccache = path.join(root, 'sccache');
  fs.writeFileSync(fakeSccache, `#!${process.execPath}\nimport fs from 'node:fs';\nconst arg=process.argv[2];\nfs.appendFileSync(process.env.FAKE_SCCACHE_LOG,arg+'\\n');\nif(arg==='--version') console.log(process.env.FAKE_SCCACHE_VERSION || 'sccache 0.19.0');\nif(arg===process.env.FAKE_SCCACHE_FAIL)process.exit(1);\n`, {mode:0o700});
  const inherited = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GITHUB_|SOLAR_|SCCACHE_|CARGO_|RUNNER_|Image)/.test(key)));
  const env = {...inherited, GITHUB_ACTIONS:'true',GITHUB_REPOSITORY:'rkw6086/solar',RUNNER_OS:nativeRunner,
    RUNNER_TEMP:root, CARGO_HOME:path.join(root,'cargo'), GITHUB_ENV:path.join(root,'env'),GITHUB_OUTPUT:path.join(root,'output'),
    ImageOS:'test-image',ImageVersion:'20261120.1',SOLAR_CACHE_INPUTS:'b'.repeat(64),SOLAR_RELEASE_SHA:sha,
    SCCACHE_PATH:fakeSccache,FAKE_SCCACHE_LOG:commandLog};
  function run(mode='prepare', changes={}) {
    for(const file of [env.GITHUB_ENV,env.GITHUB_OUTPUT])fs.rmSync(file,{force:true});
    return spawnSync(process.execPath,[script,mode],{env:{...env,...changes},encoding:'utf8',timeout:15_000});
  }
  const read = file => fs.existsSync(file)?fs.readFileSync(file,'utf8'):'';
  try { callback({root,env,run,read,commandLog}); }
  finally {fs.rmSync(root,{recursive:true,force:true});}
}

// Solar builds on native macOS and checks on Linux; the fixed Windows Cargo
// path belongs to a future Windows build and is not touched by these fixtures.
const nativeTest = process.platform === 'win32' ? test.skip : test;

nativeTest('cache archives contain only unsigned compiler, dependency, and tool paths',()=>fixture(({root,env,run,read})=>{
  const result=run();assert.equal(result.status,0,result.stderr);
  const output=read(env.GITHUB_OUTPUT), exported=read(env.GITHUB_ENV);
  const archive=output.split('paths<<SOLAR_CACHE_PATHS\n')[1].split('\nSOLAR_CACHE_PATHS')[0].split('\n');
  assert.equal(archive.length,6);
  assert.ok(archive.includes(path.join(root,'solar-ci-cache-v1','sccache')));
  assert.ok(archive.includes(path.join(env.CARGO_HOME,'registry','cache')));
  for(const item of archive)assert.doesNotMatch(item,/(?:credentials|signing|keychain|artifacts|solar-build|\.solar)/);
  assert.match(exported,/SCCACHE_CACHE_SIZE=2G\n/);
  assert.match(exported,/SCCACHE_GHA_ENABLED=false\n/);
  assert.match(exported,/RUSTC_WRAPPER=\n/);
}));

nativeTest('compatible sources share a restore prefix but retain source-specific keys',()=>fixture(({env,run,read})=>{
  assert.equal(run().status,0);const initial=read(env.GITHUB_OUTPUT);
  assert.equal(run('prepare',{SOLAR_RELEASE_SHA:'c'.repeat(40)}).status,0);const next=read(env.GITHUB_OUTPUT);
  const field=(text,key)=>text.split('\n').find(line=>line.startsWith(key+'=')).slice(key.length+1);
  assert.equal(field(initial,'prefix'),field(next,'prefix'));
  assert.notEqual(field(initial,'key'),field(next,'key'));
  assert.ok(field(next,'key').endsWith('c'.repeat(40)));
  assert.equal(run('prepare',{ImageVersion:'20261121.1'}).status,0);
  assert.notEqual(field(initial,'prefix'),field(read(env.GITHUB_OUTPUT),'prefix'));
  assert.equal(run('prepare',{SOLAR_CACHE_INPUTS:'d'.repeat(64)}).status,0);
  assert.notEqual(field(initial,'prefix'),field(read(env.GITHUB_OUTPUT),'prefix'));
}));

nativeTest('cache identity rejects foreign repositories, platforms, malformed sources, and command paths',()=>fixture(({run})=>{
  for(const change of [{GITHUB_ACTIONS:'false'},{GITHUB_REPOSITORY:'other/solar'},{RUNNER_OS:'wrong-platform'},
    {SOLAR_RELEASE_SHA:'not-a-sha'},{SOLAR_CACHE_INPUTS:'wrong'},{ImageOS:'bad\nimage'},{RUNNER_TEMP:'relative'},
    {CARGO_HOME:'relative'},{GITHUB_OUTPUT:'relative'}]) {
    const result=run('prepare',change);
    assert.notEqual(result.status,0,JSON.stringify(change));
  }
}));

nativeTest('cache setup refuses linked directories',()=>fixture(({root,run})=>{
  const foreign=path.join(root,'foreign');fs.mkdirSync(foreign);
  fs.symlinkSync(foreign,path.join(root,'solar-ci-cache-v1'),'dir');
  const result=run();assert.notEqual(result.status,0);assert.match(result.stderr,/real cache directory/);
  assert.deepEqual(fs.readdirSync(foreign),[]);
}));

nativeTest('compiler activation verifies the pinned binary before enabling its wrapper',()=>fixture(({env,run,read,commandLog})=>{
  assert.equal(run('activate').status,0);
  assert.equal(read(commandLog),'--version\n--start-server\n--zero-stats\n');
  assert.match(read(env.GITHUB_ENV),/RUSTC_WRAPPER=sccache\n/);
  fs.rmSync(commandLog);
  const bad=run('activate',{FAKE_SCCACHE_VERSION:'sccache 0.1.0'});
  assert.notEqual(bad.status,0);
  assert.equal(read(commandLog),'--version\n');
  assert.equal(read(env.GITHUB_ENV),'');
}));

nativeTest('failed activation shuts the server down and leaves the wrapper disabled',()=>fixture(({env,run,read,commandLog})=>{
  const result=run('activate',{FAKE_SCCACHE_FAIL:'--zero-stats'});
  assert.notEqual(result.status,0);
  assert.equal(read(commandLog),'--version\n--start-server\n--zero-stats\n--stop-server\n');
  assert.equal(read(env.GITHUB_ENV),'');
}));

nativeTest('cache finish always stops the active server even if statistics fail',()=>fixture(({run,read,commandLog})=>{
  const result=run('finish',{RUSTC_WRAPPER:'sccache',FAKE_SCCACHE_FAIL:'--show-stats'});
  assert.notEqual(result.status,0);
  assert.equal(read(commandLog),'--show-stats\n--stop-server\n');
}));

nativeTest('cache finish is harmless when the optional compiler wrapper was unavailable',()=>fixture(({run,read,commandLog})=>{
  const result=run('finish');assert.equal(result.status,0);
  assert.equal(read(commandLog),'');
}));
