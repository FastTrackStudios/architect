// The Board page's worker (pages/board.tsx spawns it): counts primes in a
// QuickJS runtime of its own, on its own thread (in a browser, a Web
// Worker of its own), telling the page how far it is. It has no app —
// only messages.
import { post } from "architect/worker";

export default function count({ below }) {
  let found = 0;
  for (let n = 2; n < below; n++) {
    let prime = true;
    for (let d = 2; d * d <= n; d++) {
      if (n % d === 0) {
        prime = false;
        break;
      }
    }
    if (prime) found++;
    if (n % 100000 === 0) post({ at: n });
  }
  return found;
}
