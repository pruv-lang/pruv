// Resolve the reference's extensionless relative imports to the .ts files next to them.
import { registerHooks } from 'node:module';
registerHooks({
  resolve(specifier, context, next) {
    if (specifier.startsWith('.') && !/\.[cm]?[jt]s$/.test(specifier)) {
      try { return next(specifier + '.ts', context); } catch { /* fall through */ }
    }
    return next(specifier, context);
  },
});
