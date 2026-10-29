module Demo

class Runner

def self.call( value=1 )

	parts=[ 'héllo' ,value+1 ];
	hash={:answer=>parts[ 1 ]}
	if value>0
	  parts.map{|entry|entry.to_s}.join( ',' ) + hash[:answer].to_s
    end

end
def self.second

  return ( 'done' )

end
    def self.raise_error
      fail 'controlled'
    end

end

end


puts [Demo::Runner.call, Demo::Runner.second].inspect
begin
  Demo::Runner.raise_error
rescue => error
  puts "#{ 'message'.upcase }: #{error.message}"
end
