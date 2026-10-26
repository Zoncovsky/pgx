User.find_or_create_by!(email: "seed@example.com") { |u| u.name = "Seed" }
