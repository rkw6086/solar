import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { checkGitHubBuild, githubContext } from './ci/check-github-build.mjs';
const sha = 'a'.repeat(40);
const previous = 'b'.repeat(40);
const base = { GITHUB_SERVER_URL:'https://github.com', GITHUB_REPOSITORY:'rkw6086/solar',
  GITHUB_EVENT_NAME:'workflow_dispatch', GITHUB_REF:'refs/heads/main', GITHUB_SHA:sha,
  GITHUB_TOKEN:'test-token', SOLAR_JOB_ID:'123-1' };
const beta = (commit=previous) => ({id:7,tag_name:'beta',prerelease:true,draft:false,target_commitish:commit,
  body:`<!-- solar-beta:v1 commit=${commit} -->`,assets:[{name:`Solar_beta_${commit}_aarch64.dmg`,size:123},{name:'SHA256SUMS',size:100}]});
async function fixture(run, { env: extra={}, current=null, head=sha, dirty=false, tags='0.1.0\n0.1.3', event={}, foreignTag=null, stages=[], remoteReleaseTag=sha }={}) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(),'solar-github-test-'));
  const env = {...base, RUNNER_TEMP:directory, GITHUB_OUTPUT:path.join(directory,'output'),GITHUB_ENV:path.join(directory,'env'),...extra};
  const calls=[];
  const git = (...args) => {
    if(args[0]==='status')return dirty?' M file':'';
    if(args[0]==='rev-parse')return sha;
    if(args[0]==='merge-base')return '';
    if(args[0]==='tag')return tags;
    if(args[0]==='log')return 'aabbccd Improve Sprites';
    throw new Error('Unexpected git '+args.join(' '));
  };
  const api=async (url,missing)=>{
    calls.push(url);
    if(url.startsWith('/releases?'))return [...(current?[current]:[]), ...stages];
    if(url==='/releases/7')return current;
    if(url==='/branches/main')return {commit:{sha:head}};
    if(url==='/git/ref/tags/beta')return current||foreignTag?{ref:'refs/tags/beta',object:{type:'commit',sha:foreignTag??current.target_commitish}}:null;
    if(url.startsWith('/git/ref/tags/beta-stage-'))return {ref:'refs/'+decodeURIComponent(url.slice('/git/ref/'.length)),object:{type:'commit',sha}};
    if(url==='/git/ref/tags/0.1.3')return {ref:'refs/tags/0.1.3',object:{type:'commit',sha:remoteReleaseTag}};
    throw new Error('Unexpected API '+url);
  };
  try { await run({env,directory,event,git,api,versions:(_dir,expected)=>{
    if(expected && expected!=='0.1.3')throw new Error('Version mismatch');
    return '0.1.3';
  },notesForVersion:()=>'- Stable changes.'}, calls); }
  finally { fs.rmSync(directory,{recursive:true,force:true}); }
}
test('GitHub context is restricted to Solar and supported event refs',()=>{
  assert.equal(githubContext(base,{}),'beta');
  for(const env of [{...base,GITHUB_REPOSITORY:'stranger/solar'},{...base,GITHUB_SERVER_URL:'https://example.invalid'},
    {...base,GITHUB_API_URL:'https://evil.example'},{...base,GITHUB_REF:'refs/heads/topic'},
    {...base,GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/beta'}]) assert.throws(()=>githubContext(env,{}));
});
test('stable signing requires a non-deleted exact version tag push',()=>{
  const env={...base,GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/0.1.3'};
  assert.equal(githubContext(env,{after:sha}),'release');
  assert.throws(()=>githubContext(env,{deleted:true,after:sha}));
  assert.throws(()=>githubContext(env,{after:'0'.repeat(40)}));
  assert.throws(()=>githubContext({...env,GITHUB_REF:'refs/tags/v0.1.3'},{after:sha}));
  assert.throws(()=>githubContext({...env,GITHUB_EVENT_NAME:'workflow_dispatch'},{after:sha}));
});
test('manual beta creates bounded notes and exact source outputs',async()=>fixture(async(options,calls)=>{
  const result=await checkGitHubBuild(options);
  assert.equal(result.channel,'beta');assert.equal(result.sha,sha);assert.equal(result.previous,previous);assert.equal(result.previousId,'7');
  assert.match(fs.readFileSync(options.env.GITHUB_OUTPUT,'utf8'),new RegExp(`beta_id=${sha}`));
  assert.match(fs.readFileSync(options.env.GITHUB_ENV,'utf8'),/SOLAR_PREVIOUS_BETA_RELEASE_ID=7/);
  assert.ok(calls.includes('/branches/main'));
},{current:beta()}));
test('unchanged healthy scheduled beta skips while manual rebuilds',async()=>{
  await fixture(async options=>assert.equal((await checkGitHubBuild(options)).build,false),{current:beta(sha),env:{GITHUB_EVENT_NAME:'schedule'}});
  await fixture(async options=>assert.equal((await checkGitHubBuild(options)).build,true),{current:beta(sha)});
});
test('missing beta attachment repairs an unchanged scheduled source',async()=>{
  const current=beta(sha);current.assets.pop();
  await fixture(async options=>assert.equal((await checkGitHubBuild(options)).build,true),{current,env:{GITHUB_EVENT_NAME:'schedule'}});
});
test('dirty, stale and foreign beta states are rejected',async()=>{
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/clean/),{dirty:true});
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/current GitHub main/),{head:previous});
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/unowned beta/),{foreignTag:previous});
  const current=beta();current.body='Manual release';
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/existing beta/),{current});
});
test('fork pull requests validate without any token or release API access',async()=>fixture(async(options,calls)=>{
  assert.equal((await checkGitHubBuild(options)).channel,'ci');assert.deepEqual(calls,[]);
},{env:{GITHUB_EVENT_NAME:'pull_request',GITHUB_REF:'refs/pull/9/merge',GITHUB_TOKEN:''},event:{pull_request:{base:{ref:'main',repo:{full_name:'rkw6086/solar'}}}}}));
test('pull requests must target the trusted default branch',()=>{
  assert.throws(()=>githubContext({...base,GITHUB_EVENT_NAME:'pull_request'},{pull_request:{base:{ref:'topic',repo:{full_name:'rkw6086/solar'}}}}));
});
test('stable tags bind exact version and cannot publish below a newer tag',async()=>{
  await fixture(async options=>assert.equal((await checkGitHubBuild(options)).channel,'release'),{env:{GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/0.1.3'}});
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/newer stable/),{env:{GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/0.1.3'},tags:'0.1.3\n0.1.4'});
});
test('a remote version tag moved away from its triggering source is rejected',async()=>{
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/tag or source changed/),{
    env:{GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/0.1.3'},remoteReleaseTag:previous,
  });
});
test('published stable is skipped only for an owned exact source',async()=>{
  const current={id:7,tag_name:'0.1.3',draft:false,prerelease:false,target_commitish:sha,body:`<!-- solar-release:v1 tag=0.1.3 commit=${sha} -->`};
  await fixture(async options=>assert.equal((await checkGitHubBuild(options)).build,false),{current,env:{GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/0.1.3'}});
  current.body='Foreign';
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/does not match/),{current,env:{GITHUB_EVENT_NAME:'push',GITHUB_REF:'refs/tags/0.1.3'}});
});

test('owned staged beta recovers its orphan alias after interrupted cutover',async()=>{
  const stage={...beta(sha), id:10,tag_name:`beta-stage-${sha}-0123456789abcdef`,body:`<!-- solar-beta:v1 commit=${sha} -->\n<!-- solar-beta-transition:v1 previous=${previous} release=7 -->`};
  await fixture(async options=>{const result=await checkGitHubBuild(options);assert.equal(result.previous,previous);assert.equal(result.previousId,'10');},{foreignTag:previous,stages:[stage]});
  stage.body=`<!-- solar-beta:v1 commit=${sha} -->`;
  await fixture(async options=>assert.rejects(checkGitHubBuild(options),/unowned beta/),{foreignTag:previous,stages:[stage]});
});
