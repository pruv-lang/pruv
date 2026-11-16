import { registerHooks } from 'node:module';
registerHooks({resolve(s,c,next) {
 if(s==='../engine-children'||s==='../windows-system') return next(new URL('./stubs.ts',import.meta.url).href,c);
 if(s.startsWith('.')&&!/\.[cm]?[jt]s$/.test(s)) return next(s+'.ts',c);
 return next(s,c);
}});
