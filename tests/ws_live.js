const WebSocket=require('ws');
const B='http://127.0.0.1:3111';
async function api(method,path,body,cookie){
  const r=await fetch(B+path,{method,headers:{'content-type':'application/json','x-kourt':'1',...(cookie?{cookie}:{})},body:body?JSON.stringify(body):undefined});
  const sc=r.headers.get('set-cookie'); return {s:r.status,j:await r.json().catch(()=>({})),cookie:sc?sc.split(';')[0]:cookie};
}
let fails=0; const ok=(n,c,x)=>{console.log((c?'PASS ':'FAIL ')+n+(c?'':' '+JSON.stringify(x)));if(!c)fails++};
const open=(url,cookie,origin)=>new Promise(res=>{
  const w=new WebSocket(url,{headers:{...(cookie?{cookie}:{}),...(origin?{origin}:{})}});
  const msgs=[]; w.on('message',d=>msgs.push(JSON.parse(d)));
  w.on('open',()=>res({w,msgs,code:101})); w.on('unexpected-response',(q,r)=>res({code:r.statusCode}));
  w.on('error',()=>{});
});
const wait=ms=>new Promise(r=>setTimeout(r,ms));
(async()=>{
  const tag=Date.now()%100000;
  const k=await api('POST','/api/signup',{username:'wk'+tag,password:'correct horse 9'});
  const a=await api('POST','/api/signup',{username:'wa'+tag,password:'correct horse 9'});
  const x=await api('POST','/api/signup',{username:'wx'+tag,password:'correct horse 9'});
  const c=await api('POST','/api/cases',{title:'Socket case',charge:'Talking in the library',defendant_name:'A',tier:'snack',preset:'quick'},k.cookie);
  const id=c.j.id; const snap=(await api('GET','/api/cases/'+id,null,k.cookie)).j;
  const WS=`ws://127.0.0.1:3111/api/cases/${id}/ws`;
  ok('ws without login refused', (await open(WS)).code===401);
  ok('ws as non-participant refused', (await open(WS,x.cookie)).code===404);
  ok('ws from another website refused', (await open(WS,k.cookie,'http://evil.example')).code===403);
  const pk=await open(WS,k.cookie,'http://127.0.0.1:3111');
  ok('plaintiff connects and gets snapshot', pk.code===101 && (await wait(200),pk.msgs.length===1 && pk.msgs[0].me.role==='plaintiff'), pk.msgs);
  const pa=await open(WS,k.cookie,'http://127.0.0.1:3111'); await wait(100);
  await api('POST','/api/join/'+snap.invites.defendant,null,a.cookie);
  await wait(300);
  const last=pk.msgs[pk.msgs.length-1];
  ok('plaintiff is pushed the phase change live', last.phase==='arguments' && last.defendant==='wa'+tag, last.phase);
  const pd=await open(WS,a.cookie,'http://127.0.0.1:3111'); await wait(200);
  ok('defendant socket gets a defendant-shaped snapshot', pd.msgs[0].me.role==='defendant' && !('defendant' in pd.msgs[0].invites) && 'defendant_lawyer' in pd.msgs[0].invites, pd.msgs[0].invites);
  const before=pd.msgs.length;
  await api('POST','/api/cases/'+id+'/messages',{kind:'statement',body:'Live message!'},k.cookie);
  await wait(300);
  ok('statement is pushed to the other side instantly', pd.msgs.length>before && pd.msgs[pd.msgs.length-1].messages.some(m=>m.body==='Live message!'));
  ok('second plaintiff tab updates too', pa.msgs[pa.msgs.length-1].messages.length===1);
  [pk,pa,pd].forEach(p=>p.w.close());
  console.log('\nFAILURES:',fails); process.exit(fails?1:0);
})();
