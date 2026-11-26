import type { CartLine, Product } from '$lib/types';

class CartStore {
  lines = $state<CartLine[]>([]);

  get count(): number {
    return this.lines.reduce((total, line) => total + line.qty, 0);
  }

  get subtotal_minor(): number {
    return this.lines.reduce((total, line) => total + line.total_minor, 0);
  }

  add(product: Product, qty = 1): void {
    const existing = this.lines.find((line) => line.product.id === product.id);
    if (existing) {
      existing.qty += qty;
      existing.total_minor = existing.qty * existing.unit_minor;
      return;
    }
    this.lines.push({
      product,
      qty,
      unit_minor: product.price_minor,
      total_minor: product.price_minor * qty
    });
  }

  setQty(productId: string, qty: number): void {
    if (qty <= 0) {
      this.lines = this.lines.filter((line) => line.product.id !== productId);
      return;
    }
    const line = this.lines.find((entry) => entry.product.id === productId);
    if (line) {
      line.qty = qty;
      line.total_minor = qty * line.unit_minor;
    }
  }

  clear(): void {
    this.lines = [];
  }
}

export const cart = new CartStore();
