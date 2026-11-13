// B2-ID-018: before this fix, `interface_declaration` was never
// extracted as a symbol, so IFoo/IBar below were invisible and the
// `implements` edges from Impl never resolved.

export interface IFoo {
    foo(): void;
}

export interface IBar {
    bar(): void;
}

export class Impl implements IFoo, IBar {
    foo(): void {}
    bar(): void {}
}
