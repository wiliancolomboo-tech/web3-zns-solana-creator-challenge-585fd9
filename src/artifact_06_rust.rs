// Seeds prevents cross-chain and cross-namespace collisions
   seeds = [
       b"subdomain",
       root_domain.key().as_ref(),
       hash(sub_label.as_bytes()).to_bytes().as_ref()
   ]