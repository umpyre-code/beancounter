CREATE TYPE TRANSACTION_TYPE AS ENUM ( 'debit',
  'credit',
  'promo_credit'
);

CREATE TYPE TRANSACTION_REASON AS ENUM ( 'message_read',
  'message_unread',
  'message_sent',
  'credit_added',
  'payout'
);

