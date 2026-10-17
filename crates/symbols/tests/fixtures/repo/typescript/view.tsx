export interface Props {
  label: string;
}

export function Button(props: Props) {
  return <button>{props.label}</button>;
}

export const Link = (props: Props) => <a>{props.label}</a>;
