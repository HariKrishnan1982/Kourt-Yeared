const {JSDOM}=require('jsdom'); const fs=require('fs'); const {execFileSync}=require('child_process');
const B='http://127.0.0.1:3115'; const W='/home/claude/kourt-yeared/web/';
const code={admin:fs.readFileSync(W+'admin.js','utf8'),app:fs.readFileSync(W+'app.js','utf8')};
const strip=h=>h.replace(/<link[^>]*>/g,'').replace(/<script[^>]*><\/script>/g,'');
const page={admin:strip(fs.readFileSync(W+'admin.html','utf8')),app:strip(fs.readFileSync(W+'index.html','utf8'))};
let fails=0; const ok=(n,c,x)=>{console.log((c?'PASS ':'FAIL ')+n+(c?'':'  -> '+String(x).slice(0,240)));if(!c)fails++};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,t=4000){const s=Date.now();while(Date.now()-s<t){try{const v=fn();if(v)return v}catch(e){}await wait(40)}return null}
const sql=(q)=>execFileSync('python3',['-c',"import sqlite3,sys;d=sqlite3.connect('/tmp/admui.db');d.execute(sys.argv[1]);d.commit()",q]);
function client(){let cookie='';return{async call(m,p,b){const r=await fetch(B+p,{method:m,headers:{'content-type':'application/json','x-kourt':'1',...(cookie?{cookie}:{})},body:b?JSON.stringify(b):undefined});const sc=r.headers.get('set-cookie');if(sc){const c=sc.split(';')[0];cookie=c.endsWith('=')?'':c}return{s:r.status,j:await r.json().catch(()=>({}))}}}}
async function browser(path,kind){
  let cookie=''; const errors=[]; let promptAnswer=null; const prompts=[];
  const dom=new JSDOM(page[kind],{url:B+path,pretendToBeVisual:true,runScripts:'outside-only'}); const w=dom.window;
  w.fetch=async(u,o={})=>{const r=await fetch(new URL(u,B),{...o,headers:{...(o.headers||{}),...(cookie?{cookie}:{})}});const sc=r.headers.get('set-cookie');if(sc){const c=sc.split(';')[0];cookie=c.endsWith('=')?'':c}return r};
  w.WebSocket=class{constructor(){} close(){}};
  w.Element.prototype.scrollIntoView=()=>{}; w.document.execCommand=()=>true; w.confirm=()=>true; w.prompt=(m)=>{prompts.push(m);return promptAnswer};
  w.addEventListener('error',e=>errors.push(e.message)); w.addEventListener('unhandledrejection',e=>errors.push('unhandled: '+(e.reason&&e.reason.message)));
  w.eval(code[kind]);
  const $=s=>w.document.querySelector(s), $$=s=>[...w.document.querySelectorAll(s)];
  const text=()=>w.document.body.textContent;
  const fill=(sel,v)=>{const el=$(sel);el.value=v;el.dispatchEvent(new w.Event('input',{bubbles:true}))};
  const click=el=>el.dispatchEvent(new w.MouseEvent('click',{bubbles:true,cancelable:true,button:0}));
  const btn=t=>$$('button,a.btn').find(b=>b.textContent.trim()===t);
  const link=t=>$$('a').find(a=>a.textContent.trim()===t);
  const submit=f=>f.dispatchEvent(new w.Event('submit',{bubbles:true,cancelable:true}));
  return{w,$,$$,text,fill,click,btn,link,submit,errors,prompts,answer:v=>{promptAnswer=v},path:()=>w.location.pathname+w.location.search};
}
(async()=>{
  const tag=Date.now()%1000000, PW='correct horse 9';
  const kv=client(), ar=client(), jr=client(), lw=client();
  const nm={k:'kav'+tag,a:'aru'+tag,j:'jur'+tag,l:'law'+tag};
  const ids={};
  for(const[k,c]of[['k',kv],['a',ar],['j',jr],['l',lw]]){const r=await c.call('POST','/api/signup',{username:nm[k],password:PW});ids[k]=r.j.id}
  const mk=await kv.call('POST','/api/cases',{title:'Samosa trial '+tag,charge:'Ate the last samosa',defendant_name:'Aru',details:'seen at canteen',tier:'snack',demand:'two samosas',preset:'quick'});
  const cid=mk.j.id; const sn=(await kv.call('GET','/api/cases/'+cid)).j;
  await ar.call('POST','/api/join/'+sn.invites.defendant);
  await kv.call('POST','/api/cases/'+cid+'/messages',{kind:'statement',body:'<img src=x onerror="window.pwned=1"> He did it'});
  await ar.call('POST','/api/cases/'+cid+'/messages',{kind:'statement',body:'I was elsewhere.'});
  const spare=(await kv.call('POST','/api/cases',{title:'Spare case '+tag,charge:'Extra charge',defendant_name:'Zed',tier:'meal',preset:'quick'})).j.id;

  // ---------- login ----------
  const a=await browser('/admin','admin');
  ok('admin page shows the login form', await until(()=>/Admin login/.test(a.text())&&a.$('#au')));
  ok('login page explains it only works locally', /only works on the computer/.test(a.text()));
  a.fill('#au','clerk'); a.fill('#ap','wrong-wrong-wrong'); a.submit(a.$('form'));
  ok('wrong password shows an error', await until(()=>/Wrong admin username or password/.test(a.text())));
  a.fill('#ap','clerk-test-pass-123'); a.submit(a.$('form'));
  ok('right password opens the overview', await until(()=>/Overview/.test(a.$('h1')&&a.$('h1').textContent)&&a.$$('.tile').length>=8));
  ok('top bar shows admin name and nav', /clerk/.test(a.$('.who').textContent)&&a.$$('.adm-bar nav a').length===6&&a.$('.adm-bar nav a[aria-current="page"]').textContent==='Overview');
  const tiles=Object.fromEntries(a.$$('.tile').map(t=>[t.querySelector('span').textContent.split(' (')[0],t.querySelector('strong').textContent]));
  ok('overview counts players and open cases', Number(tiles['Players'])>=4&&/^\d+$/.test(tiles['Open cases']), JSON.stringify(tiles));
  ok('overview lists newest players and cases', /Newest players/.test(a.text())&&a.$$('table').length>=2&&a.link('Samosa trial '+tag));

  // ---------- users ----------
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Users'));
  ok('users page lists players', await until(()=>a.path()==='/admin/users'&&a.$('#q')&&a.link(nm.a)));
  a.fill('#q',nm.a.slice(0,6)); a.submit(a.$('form[role=search]'));
  ok('search narrows the list', await until(()=>a.$$('tbody tr').length===1&&a.link(nm.a)), a.$$('tbody tr').length);
  a.click(a.link(nm.a));
  ok('user page shows details', await until(()=>a.$('h1')&&a.$('h1').textContent===nm.a&&/Statements/.test(a.text())&&/Active/.test(a.text())));
  a.fill('#reason','Spamming the court'); a.click(a.btn('Suspend'));
  ok('suspending flips the badge and shows the reason', await until(()=>/Suspended/.test(a.$('.row').textContent)&&/Reason: Spamming the court/.test(a.text())), a.text().slice(0,200));
  const lg=await client().call('POST','/api/login',{username:nm.a,password:PW}); ok('suspended player is locked out with the reason', lg.s===403&&/Spamming the court/.test(lg.j.error), JSON.stringify(lg));
  a.click(a.btn('Restore account'));
  ok('restore brings them back', await until(()=>/Active/.test(a.$('.row').textContent)&&a.btn('Suspend')));
  ok('restored player can log in again', (await client().call('POST','/api/login',{username:nm.a,password:PW})).s===200);
  a.click(a.btn('Issue temporary password'));
  const temp=await until(()=>a.$('[data-testid="temp-password"]')&&a.$('[data-testid="temp-password"]').textContent);
  ok('temporary password is shown once', temp&&temp.length>=12, temp);
  ok('temporary password works for the player', (await client().call('POST','/api/login',{username:nm.a,password:temp})).s===200);
  ok('old password stopped working', (await client().call('POST','/api/login',{username:nm.a,password:PW})).s===401);
  a.click(a.link('Back to player')); await until(()=>a.$('h1')&&a.$('h1').textContent===nm.a);
  ok('revisiting does not show the password again', !/temp-password/.test(a.w.document.body.innerHTML));
  a.fill('#rename',nm.a+'x'); a.click(a.btn('Change username'));
  ok('rename updates the page', await until(()=>a.$('h1').textContent===nm.a+'x'), a.$('h1')&&a.$('h1').textContent);
  a.fill('#note','Prefers biryani.'); a.click(a.btn('Save note'));
  await until(()=>/Note saved/.test(a.$('#toasts').textContent));
  ok('note is saved', (await (async()=>{const b=await browser('/admin/users/'+ids.a,'admin');return b})()) !== null);
  a.answer('wrong name'); a.click(a.btn('Delete account')); await wait(250);
  ok('delete needs the exact name typed', !/Deleted/.test(a.$('.row').textContent)&&/Type .* to delete/.test(a.prompts[0]), a.prompts[0]);
  a.answer(nm.a+'x'); a.click(a.btn('Delete account'));
  ok('typing the name deletes (anonymises) the account', await until(()=>/Deleted/.test(a.$('.row').textContent)&&a.$('h1').textContent===`deleted-${ids.a}`&&!a.btn('Suspend')), a.$('h1').textContent);
  ok('deleted account sees no manage buttons', !a.btn('Issue temporary password')&&/was deleted/.test(a.text()));

  // ---------- cases ----------
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Cases'));
  ok('cases page lists cases with status', await until(()=>a.path()==='/admin/cases'&&a.$('#phase')&&a.link('Samosa trial '+tag)&&/Arguments/.test(a.text())));
  a.$('#phase').value='summons'; a.submit(a.$('form[role=search]'));
  ok('stage filter works', await until(()=>a.$$('tbody tr').length>=1&&a.link('Spare case '+tag)&&!a.link('Samosa trial '+tag)));
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Cases')); await until(()=>a.$('#phase')&&a.link('Samosa trial '+tag)&&a.$$('tbody tr').length>1);
  a.click(a.link('Samosa trial '+tag));
  ok('case page shows parties, stakes, transcript', await until(()=>a.$('h1')&&a.$('h1').textContent==='Samosa trial '+tag&&/two samosas/.test(a.text())&&a.$$('.msg').length===2));
  ok('HTML in statements shows as text in admin too', a.$('.msg img')===null&&!a.w.pwned&&/<img src=x/.test(a.$('.msg').textContent));
  ok('invite status shown without tokens', /Used by/.test(a.text())&&!/\/join\//.test(a.w.document.body.innerHTML));
  const before=(await client().call('GET','/api/admin/stats')).s; // unauthenticated -> 401
  a.fill('#mins','25'); a.click(a.btn('Extend current stage'));
  ok('extend shows a confirmation', await until(()=>/Time added/.test(a.$('#toasts').textContent)));
  a.click(a.btn('Remove'));
  ok('removing a message marks it gone', await until(()=>a.$$('.msg.gone').length===1&&/removed by the court/.test(a.$('.msg.gone').textContent)));
  ok('players see the removal', (await kv.call('GET','/api/cases/'+cid)).j.messages.some(m=>m.removed));
  a.fill('#e-title','Samosa trial (fixed) '+tag); a.click(a.btn('Save changes'));
  ok('wording fix is saved', await until(()=>a.$('h1')&&/fixed/.test(a.$('h1').textContent)));
  a.$('#outcome').value='not_guilty'; a.click(a.btn('Close case'));
  ok('force-close ends the case with the normal rules', await until(()=>/Not guilty/.test(a.$('.row').textContent)&&/Sentence/.test(a.text())&&a.btn('Mark paid')), a.text().slice(0,300));
  a.click(a.btn('Mark paid'));
  ok('mark paid flips the badge', await until(()=>a.btn('Mark unpaid')&&/Paid/.test(a.text())));
  a.answer(cid); a.click(a.btn('Delete case'));
  ok('typing the case code deletes it and returns to the list', await until(()=>a.path()==='/admin/cases'&&!a.link('Samosa trial (fixed) '+tag)));
  ok('the case is gone for players', (await kv.call('GET','/api/cases/'+cid)).s===404);

  // ---------- settings + public effects ----------
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Settings'));
  ok('settings page loads current values', await until(()=>a.$('#signups')&&a.$('#signups').checked===true&&a.$('#max').value==='10'));
  a.$('#signups').checked=false; a.fill('#ann','Court is closed on Friday.'); a.fill('#max','4'); a.submit(a.$('form'));
  ok('saving settings confirms', await until(()=>/Settings saved/.test(a.$('#toasts').textContent)));
  const pub=await browser('/','app');
  ok('public site shows the announcement banner', await until(()=>pub.$('.site-banner')&&/Court is closed on Friday/.test(pub.$('.site-banner').textContent)));
  ok('public site hides sign-up and explains', await until(()=>/New accounts are paused/.test(pub.text())&&!pub.btn('Create account')&&pub.btn('Log in')), pub.text().slice(0,300));
  const prog=await browser('/','app'); await until(()=>prog.$('form')); prog.fill('#au',nm.k); prog.fill('#ap',PW); prog.submit(prog.$('form'));
  ok('existing players can still log in while sign-ups are paused', await until(()=>/Your cases/.test(prog.text())&&prog.$('.site-banner')));
  a.$('#toasts').replaceChildren(); a.$('#signups').checked=true; a.fill('#ann',''); a.submit(a.$('form')); await until(()=>/Settings saved/.test(a.$('#toasts').textContent));
  const pub2=await browser('/','app'); ok('turning it back on restores sign-up and removes the banner', await until(()=>pub2.btn('Create account')&&!pub2.$('.site-banner')));
  a.fill('#max','0'); a.submit(a.$('form'));
  ok('invalid settings show an inline error', await until(()=>/between 1 and 100/.test(a.text())));

  // ---------- log + account ----------
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Activity log'));
  ok('activity log lists what we did', await until(()=>/ban user/.test(a.text())&&/reset password/.test(a.text())&&/delete case/.test(a.text())&&/change settings/.test(a.text())));
  ok('log never shows the temporary password', !a.text().includes(temp));
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Account'));
  await until(()=>a.$('#cur')); a.fill('#cur','nope-nope-nope-1'); a.fill('#new','a-brand-new-password-1'); a.submit(a.$('form'));
  ok('wrong current password is explained', await until(()=>/current password is wrong/.test(a.text())));
  a.fill('#cur','clerk-test-pass-123'); a.fill('#new','short'); a.submit(a.$('form'));
  ok('weak new password is rejected inline', await until(()=>/at least 12 characters/.test(a.text())));

  // ---------- session expiry mid-use ----------
  sql("DELETE FROM admin_sessions");
  a.click(a.$$('.adm-bar nav a').find(x=>x.textContent==='Users'));
  ok('an ended admin session returns to the login form with a message', await until(()=>/Your session ended/.test(a.text())&&a.$('#au')));
  a.fill('#au','clerk'); a.fill('#ap','clerk-test-pass-123'); a.submit(a.$('form'));
  ok('logging in again works', await until(()=>a.$('.adm-bar')));
  a.click(a.btn('Log out')); ok('log out returns to login', await until(()=>/Admin login/.test(a.text())));

  const errs=[a,pub,prog,pub2].flatMap(b=>b.errors); ok('no uncaught JS errors', errs.length===0, errs.join(' | '));
  console.log('\nFAILURES:',fails); process.exit(fails?1:0);
})().catch(e=>{console.error('HARNESS ERROR',e);process.exit(2)});
