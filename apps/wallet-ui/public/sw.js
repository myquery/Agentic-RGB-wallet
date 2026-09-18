const CACHE='luma-wallet-shell-v2';
self.addEventListener('install',event=>{event.waitUntil(caches.open(CACHE).then(cache=>cache.addAll(['/','/icon-192.png','/icon-512.png','/manifest.webmanifest'])))});
self.addEventListener('activate',event=>{event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(key=>key!==CACHE).map(key=>caches.delete(key)))).then(()=>self.clients.claim()))});
self.addEventListener('fetch',event=>{
 const url=new URL(event.request.url);
 // Never cache wallet state, prompts, approvals, or payment requests. Never queue writes.
 if(event.request.method!=='GET'||url.origin!==self.location.origin||url.pathname.startsWith('/api/'))return;
 if(!['/','/icon-192.png','/icon-512.png','/manifest.webmanifest'].includes(url.pathname)&&!url.pathname.startsWith('/assets/'))return;
 event.respondWith(fetch(event.request).then(response=>{if(response.ok){const copy=response.clone();event.waitUntil(caches.open(CACHE).then(cache=>cache.put(event.request,copy)))}return response}).catch(()=>caches.match(event.request)));
});
