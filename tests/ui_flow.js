const {JSDOM}=require('jsdom'); const WS=require('ws'); const fs=require('fs'); const path=require('path'); const os=require('os'); const {execFileSync}=require('child_process');
const B='http://127.0.0.1:3111';
const W=path.resolve(__dirname, '../web')+'/';
const code=fs.readFileSync(W+'app.js','utf8');
const html=fs.readFileSync(W+'index.html','utf8').replace(/<link[^>]*>/g,'').replace(/<script[^>]*><\/script>/g,'');
const DB=process.env.KOURT_DB_FILE || (process.platform === 'win32' ? path.join(os.tmpdir(), 'ws.db') : '/tmp/ws.db');
const pyBin=process.platform === 'win32' ? 'python' : 'python3';
let fails=0; const ok=(n,c,x)=>{console.log((c?'PASS ':'FAIL ')+n+(c?'':' '+String(x).slice(0,200)));if(!c)fails++};
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,t=4000){const s=Date.now();while(Date.now()-s<t){try{const v=fn();if(v)return v}catch(e){}await wait(50)}return null}
async function browser(path){
  let cookie=''; const errors=[];
  const dom=new JSDOM(html,{url:B+path,pretendToBeVisual:true,runScripts:'outside-only'});
  const w=dom.window;
  w.fetch=async(u,o={})=>{const r=await fetch(new URL(u,B),{...o,headers:{...(o.headers||{}),...(cookie?{cookie}:{})}});const sc=r.headers.get('set-cookie');if(sc){const c=sc.split(';')[0];cookie=c.endsWith('=')?'':c}return r};
  w.WebSocket=class{constructor(u){this.ws=new WS(u,{headers:{cookie,origin:B}});this.ws.on('open',()=>this.onopen&&this.onopen());this.ws.on('message',d=>this.onmessage&&this.onmessage({data:d.toString()}));this.ws.on('close',()=>this.onclose&&this.onclose());this.ws.on('error',()=>{})}close(){this.ws.close()}};
  w.Element.prototype.scrollIntoView=()=>{}; w.document.execCommand=()=>true; w.confirm=()=>true;
  w.addEventListener('error',e=>errors.push(e.message));
  w.eval(code);
  const $=s=>w.document.querySelector(s), $$=s=>[...w.document.querySelectorAll(s)];
  const text=()=>w.document.body.textContent;
  const fill=(sel,v)=>{const el=$(sel);el.value=v;el.dispatchEvent(new w.Event('input',{bubbles:true}))};
  const click=el=>el.dispatchEvent(new w.MouseEvent('click',{bubbles:true,cancelable:true,button:0}));
  const btn=t=>$$('button,a.btn').find(b=>b.textContent.trim()===t);
  const submit=f=>f.dispatchEvent(new w.Event('submit',{bubbles:true,cancelable:true}));
  const close=()=>{w.close()};
  return {w,$,$$,text,fill,click,btn,submit,errors,close,cookie:()=>cookie};
}
const sql=(q)=>execFileSync(pyBin,['-c',`import sqlite3,sys;d=sqlite3.connect(sys.argv[2]);d.execute(sys.argv[1]);d.commit()`,q,DB]);
(async()=>{
  const tag=Date.now()%1000000;
  // landing + signup
  const k=await browser('/');
  ok('landing renders headline and auth form', await until(()=>k.$('h1')&&/File a case/.test(k.$('h1').textContent)&&k.$('form')));
  ok('landing shows sample summons', /Summons/.test(k.text())&&/Served/.test(k.text()));
  k.fill('#au','kv'+tag); k.fill('#ap','correct horse 9'); k.submit(k.$('form'));
  ok('signup lands on home', await until(()=>/Your cases/.test(k.text())));
  ok('header shows username and log out', /kv\d+/.test(k.$('.who').textContent)&&k.btn('Log out'));
  // file a case
  k.click(k.btn('File a case')); ok('file-case form opens', await until(()=>k.$('#title')));
  ok('plan preview shown (3/6/3 for quick)', /12 min in all: 3 min .* 6 min .* 3 min/.test(k.text()), k.$('.muted.small')&&k.$('.muted.small').textContent);
  k.fill('#title','Samosa trial'); k.fill('#charge','Ate the last samosa'); k.fill('#defendant_name','Arun'); k.fill('#demand','two samosas');
  k.submit(k.$('form.sheet'));
  ok('case room opens after filing', await until(()=>/Case KY-/.test(k.text())&&/Serve the summons/.test(k.text())));
  ok('room shows parties, stakes, clock', /v\./.test(k.$('.parties').textContent)&&/two samosas/.test(k.text())&&/left/.test(k.$('#clock').textContent));
  const links=k.$$('.invite input').map(i=>i.value); ok('three invite links shown to plaintiff', links.length===3&&links.every(l=>l.startsWith(B+'/join/')), links);
  const defLink=links[0].replace(B,''); const caseId=k.w.location.pathname.split('/').pop();
  ok('whatsapp share link built', k.$$('a[href^="https://wa.me/"]').length===3&&k.$$('a[href^="https://wa.me/"]')[0].rel.includes('noopener'));
  // defendant opens link logged out -> sees summons + auth
  const a=await browser(defLink);
  ok('defendant link shows summons and signup', await until(()=>/Samosa trial/.test(a.text())&&/files? a case against you|has filed a case against you/.test(a.text())&&a.$('form')), a.text().slice(0,200));
  a.fill('#au','ar'+tag); a.fill('#ap','correct horse 9'); a.submit(a.$('form'));
  ok('after signup they return to the summons', await until(()=>a.btn('Answer the summons')));
  a.click(a.btn('Answer the summons'));
  ok('defendant enters the room', await until(()=>/Arguments/.test(a.text())&&a.$('#say')&&/You are the defendant/.test(a.text())));
  ok('plaintiff is pushed to arguments live', await until(()=>k.$('li[aria-current="step"]')&&k.$('li[aria-current="step"]').textContent==='Arguments'));
  ok('plaintiff now sees defendant name & composer', /ar\d+/.test(k.$('.parties').textContent)&&k.$('#say'));
  // arguing + XSS
  k.fill('#say','<img src=x onerror="window.pwned=1"><b>bold?</b> He took it.');
  k.click(k.btn('Submit statement'));
  ok('statement appears for the defendant live', await until(()=>a.$$('.entry.plaintiff').length===1));
  ok('HTML in statements is shown as text, not run', a.$('.entry img')===null && a.$('.entry b')===null && /<img src=x/.test(a.$('.entry.plaintiff').textContent) && !a.w.pwned);
  ok('composer cleared after sending', k.$('#say').value==='');
  a.fill('#say','Objection incoming. I was nowhere near it.'); a.click(a.btn('Object'));
  ok('objection stamp shows for both', await until(()=>k.$('.objection.defendant .stamp')&&a.$('.objection.defendant .stamp')));
  a.fill('#say','I am innocent!'); a.click(a.btn('Submit statement'));
  ok('defense statement lands in plaintiff view', await until(()=>k.$$('.entry.defendant').length===1));
  // draft survives live updates
  k.fill('#say','my half-written closing...'); a.fill('#say','another line'); a.click(a.btn('Submit statement'));
  await until(()=>k.$$('.entry.defendant').length===2);
  ok('half-typed draft survives incoming live update', k.$('#say').value==='my half-written closing...', k.$('#say').value);
  // jury
  const jl=k.$$('.invite input').map(i=>i.value).find(v=>true); const snap=await (await k.w.fetch('/api/cases/'+caseId,{headers:{'x-kourt':'1'}})).json();
  const jury=await browser('/join/'+snap.invites.jury);
  await until(()=>jury.$('form')); jury.fill('#au','jr'+tag); jury.fill('#ap','correct horse 9'); jury.submit(jury.$('form'));
  ok('juror joins via link', await until(()=>jury.btn('Join the jury'))); jury.click(jury.btn('Join the jury'));
  ok('juror listens during arguments', await until(()=>/Listen closely/.test(jury.text())&&!jury.$('#say')));
  // fast-forward clock
  sql(`UPDATE cases SET deadline_ms=1 WHERE id='${caseId}'`);
  ok('everyone moves to voting by the clock', await until(()=>jury.btn('Guilty')&&/Vote/.test(k.$('li[aria-current="step"]').textContent),5000));
  ok('parties lose the composer at voting', await until(()=>!k.$('#say')&&!a.$('#say')));
  jury.click(jury.btn('Guilty'));
  ok('verdict stamp slams in for everyone, live', await until(()=>k.$('.stamp.big')&&a.$('.stamp.big')&&jury.$('.stamp.big'),5000));
  ok('guilty verdict, 1-0, sentence shown', /Guilty/.test(k.$('.stamp.big').textContent)&&/1 to 0/.test(k.text())&&/two samosas/.test(k.$('.sentence').textContent), k.$('.verdict')&&k.$('.verdict').textContent);
  ok('stamp animates on first sight', k.$('.stamp.big').classList.contains('slam'));
  ok('only the plaintiff can mark as paid', k.btn('Mark as paid')&&!a.btn('Mark as paid')&&!jury.btn('Mark as paid'));
  k.click(k.btn('Mark as paid'));
  ok('paid status propagates to defendant', await until(()=>/Paid\. Case settled/.test(a.text())));
  // home reflects state
  a.click(a.$('.mark')); ok('defendant home lists the case with outcome', await until(()=>/Your cases/.test(a.text())&&/Guilty/.test(a.$('.cases').textContent)));
  // logged-out visit to a case page bounces to login
  const anon=await browser('/case/'+caseId); ok('anonymous case URL falls back to landing', await until(()=>anon.$('form')&&/File a case/.test(anon.text())));
  // bad link
  const bad=await browser('/join/not-a-real-token'); ok('bad invite link explains itself', await until(()=>/This link doesn.t work/.test(bad.text())));
  const errs=[k,a,jury,anon,bad].flatMap(b=>b.errors); ok('no uncaught JS errors in any browser', errs.length===0, errs.join(' | '));
  console.log('\nFAILURES:',fails); process.exit(fails?1:0);
})().catch(e=>{console.error('HARNESS ERROR',e);process.exit(2)});
