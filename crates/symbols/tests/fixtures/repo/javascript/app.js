import { helper } from './helper';

class Store {
  load(key) {
    return helper(key);
  }
}

function render(items) {
  return items.map((item) => item.name);
}

const format = (value) => `${value}`;
const notAFunction = 42;
