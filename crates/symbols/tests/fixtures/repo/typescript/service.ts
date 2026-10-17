export namespace Billing {
  export function charge(amount: number): void {}
}

export interface Account {
  id: string;
}

export type Id = string;

export enum Role {
  Admin,
  Viewer,
}

export abstract class Repository<T> {
  abstract find(id: Id): T | undefined;

  save(item: T): void {}
}

export const parse = (text: string): number => Number(text);
