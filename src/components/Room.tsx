import { ReactNode } from "react";
import { RoomVariant } from "../lib/accounts";

type Props = {
  variant: RoomVariant;
  name: string;
  email?: string | null;
  dir: string;
  children: ReactNode;
};

// One account as a sealed room: identity on top, content and actions below.
export default function Room({ variant, name, email, dir, children }: Props) {
  return (
    <article className={`room ${variant === "normal" ? "" : `room--${variant}`}`}>
      <header>
        <h3 className="room__name">{name}</h3>
        {email && <div className="room__mail">{email}</div>}
        <div className="room__dir">{dir}</div>
      </header>
      {children}
    </article>
  );
}
