export class Button {
  label: string = "";
}

export function renderButton(label: string): string {
  return `<button>${label}</button>`;
}
