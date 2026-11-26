export type Product = {
  id: string;
  name: string;
  price_minor: number;
  barcode?: string;
  sku?: string;
  category: string;
  pack_size?: number;
  pack_price_minor?: number;
};

export type Customer = {
  id: string;
  name: string;
  discount_bp: number;
  credit_limit_minor: number;
};

export type Staff = {
  id: string;
  name: string;
};

export type Catalog = {
  version: number;
  currency: string;
  cash_step: number;
  tax_bp: number;
  products: Product[];
  customers: Customer[];
  staff: Staff[];
  caps: {
    max_sale_minor: number;
    max_mobile_money_minor: number;
    total_mobile_money_minor: number;
  };
};

export type CartLine = {
  product: Product;
  qty: number;
  unit_minor: number;
  total_minor: number;
};

export type SaleLine = {
  name: string;
  qty: number;
  unit_minor: number;
};

export type Sale = {
  uid: string;
  occurred_at: string;
  total_minor: number;
  tender: 'cash' | 'mtn_momo' | 'airtel_money' | 'credit';
  reference?: string;
  customer_id?: string;
  cashier: string;
  status: 'pending' | 'synced';
  unverified: boolean;
  lines: SaleLine[];
};

export type Status = {
  paired: boolean;
  offline: boolean;
  catalog_version: number;
  last_sync: string;
  pending: number;
  battery: number;
};

export type NewSale = {
  total_minor: number;
  tender: Sale['tender'];
  reference?: string;
  customer_id?: string;
  cashier: string;
  lines: SaleLine[];
};
