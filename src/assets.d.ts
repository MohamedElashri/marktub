/// <reference types="vite/client" />
declare module "*.css";

declare module "virtual:marktub-export-fonts" {
  const fonts: Record<string, string>;
  export default fonts;
}

declare module "virtual:marktub-notices" {
  const text: string;
  export default text;
}
